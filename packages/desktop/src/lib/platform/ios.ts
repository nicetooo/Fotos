/**
 * iOS Platform Service
 * iOS 平台服务实现（WebKit 桥接）
 */

import { invoke } from "@tauri-apps/api/core";
import type {
    PlatformCapabilities,
    ImportProgress,
    ImportResult,
    PermissionStatus,
    MapTileConfig,
    IOSPlatformService,
} from "./types";

type EventHandler = { event: string; handler: EventListener };

class IOSPlatformServiceImpl implements IOSPlatformService {
    readonly platform = 'ios' as const;
    readonly isMobile = true;

    readonly capabilities: PlatformCapabilities = {
        arbitraryPathAccess: false,
        nativePhotoLibrary: true,
        runtimePermissions: true,
        revealInFileManager: false,
        folderSelection: false,
    };

    private progressCallbacks = new Set<(p: ImportProgress) => void>();
    private completeCallbacks = new Set<(r: ImportResult) => void>();
    private scanningCallbacks = new Set<(s: boolean) => void>();
    private permissionCallbacks = new Set<(hasFullAccess: boolean) => void>();
    private eventListeners: EventHandler[] = [];
    private dbPath = '';
    private thumbDir = '';
    private _hasFullAccess = false;
    private _bridgeReady = false;
    private importRunning = false;

    get hasFullPhotoAccess(): boolean {
        return this._hasFullAccess;
    }

    async initialize(dbPath: string, thumbDir: string): Promise<void> {
        this.dbPath = dbPath;
        this.thumbDir = thumbDir;

        // iOS 导入进度事件
        this.addListener('ios-import-progress', (e: CustomEvent) => {
            const detail = e.detail;
            const progress: ImportProgress = {
                success: detail.success || 0,
                failure: detail.failure || 0,
                duplicates: detail.duplicates || 0,
                current: detail.current,
                total: detail.total,
                lastPath: detail.phase || "",
            };
            this.progressCallbacks.forEach(cb => cb(progress));
        });

        // iOS 导出进度事件
        this.addListener('ios-export-progress', (e: CustomEvent) => {
            const detail = e.detail;
            const progress: ImportProgress = {
                success: 0,
                failure: 0,
                duplicates: 0,
                current: detail.current,
                total: detail.total,
                lastPath: "Exporting...",
            };
            this.progressCallbacks.forEach(cb => cb(progress));
        });

        // iOS 导入完成事件
        this.addListener('ios-import-complete', (e: CustomEvent) => {
            const detail = e.detail;
            console.log("[iOS] Import complete:", detail);
            this.scanningCallbacks.forEach(cb => cb(false));
            this.completeCallbacks.forEach(cb => cb({
                success: detail.success || 0,
                failure: detail.failure || 0,
                duplicates: detail.duplicates || 0,
            }));
        });

        // iOS 导入开始事件
        this.addListener('ios-import-started', () => {
            console.log("[iOS] Import started");
            this.scanningCallbacks.forEach(cb => cb(true));
        });

        // iOS 权限授予事件
        this.addListener('ios-permission-granted', (e: CustomEvent) => {
            const detail = e.detail;
            console.log("[iOS] Permission granted:", detail.type);
            this._hasFullAccess = detail.type === "full";
            this.permissionCallbacks.forEach(cb => cb(this._hasFullAccess));
        });

        // iOS 权限拒绝事件
        this.addListener('ios-permission-denied', (e: CustomEvent) => {
            const detail = e.detail;
            console.log("[iOS] Permission denied:", detail.message);
            this._hasFullAccess = false;
            this.permissionCallbacks.forEach(cb => cb(false));
            this.scanningCallbacks.forEach(cb => cb(false));
        });

        // iOS 权限状态检查结果
        this.addListener('ios-permission-status', (e: CustomEvent) => {
            const status = e.detail.status;
            console.log("[iOS] Permission status:", status);
            this._hasFullAccess = status === 3; // 3 = authorized (full access)
            this.permissionCallbacks.forEach(cb => cb(this._hasFullAccess));
        });

        // iOS 同步跳过事件
        this.addListener('ios-sync-skipped', (e: CustomEvent) => {
            const status = e.detail.status;
            console.log("[iOS] Sync skipped, status:", status);
            this._hasFullAccess = status === 3;
            this.permissionCallbacks.forEach(cb => cb(this._hasFullAccess));
        });

        // iOS Swift 桥接就绪事件
        this.addListener('footos-bridge-ready', () => {
            console.log("[iOS] Swift bridge is ready");
            this._bridgeReady = true;
        });

        // 检查桥接是否已就绪
        if ((window as any).__FOOTOS_BRIDGE_READY__) {
            console.log("[iOS] Swift bridge was already ready");
            this._bridgeReady = true;
        }

        // 暴露全局函数给 Swift 调用
        (window as any).__handleIOSImportPaths = (paths: string[], db: string, thumb: string) => {
            console.log("[iOS] Received paths via global function:", paths?.length);
            if (!paths || paths.length === 0) {
                this.scanningCallbacks.forEach(cb => cb(false));
                return;
            }
            this.processIOSImport(paths, db || this.dbPath, thumb || this.thumbDir);
        };
    }

    private addListener(event: string, handler: (e: CustomEvent) => void): void {
        const wrappedHandler = handler as EventListener;
        window.addEventListener(event, wrappedHandler);
        this.eventListeners.push({ event, handler: wrappedHandler });
    }

    cleanup(): void {
        this.eventListeners.forEach(({ event, handler }) => {
            window.removeEventListener(event, handler);
        });
        this.eventListeners = [];
        delete (window as any).__handleIOSImportPaths;
        this.progressCallbacks.clear();
        this.completeCallbacks.clear();
        this.scanningCallbacks.clear();
    }

    async requestImport(): Promise<void> {
        console.log("[iOS] Requesting import, bridge ready:", this._bridgeReady);

        const webkit = (window as any).webkit;
        const bridgeAvailable = webkit?.messageHandlers?.footosPhotoPicker || (window as any).__FOOTOS_BRIDGE_READY__;

        if (bridgeAvailable) {
            console.log("[iOS] Using webkit message handler");
            webkit.messageHandlers.footosPhotoPicker.postMessage({
                command: "requestAndImport",
                dbPath: this.dbPath,
                thumbDir: this.thumbDir
            });
            return;
        }

        // 桥接未就绪，等待后重试
        console.log("[iOS] Bridge not ready, waiting...");
        this.scanningCallbacks.forEach(cb => cb(true));

        const ready = await this.waitForBridge(3000);
        if (ready) {
            console.log("[iOS] Bridge became ready, proceeding");
            const webkit = (window as any).webkit;
            webkit.messageHandlers.footosPhotoPicker.postMessage({
                command: "requestAndImport",
                dbPath: this.dbPath,
                thumbDir: this.thumbDir
            });
        } else {
            console.error("[iOS] Bridge not available after timeout");
            this.scanningCallbacks.forEach(cb => cb(false));
        }
    }

    async syncPhotosIfFullAccess(): Promise<void> {
        console.log("[iOS] Checking for full access to auto-sync...");

        const ready = await this.waitForBridge(3000);
        if (ready) {
            const webkit = (window as any).webkit;
            webkit.messageHandlers.footosPhotoPicker.postMessage({
                command: "syncIfFullAccess",
                dbPath: this.dbPath,
                thumbDir: this.thumbDir
            });
        } else {
            console.log("[iOS] Bridge not ready for auto-sync");
        }
    }

    showLimitedLibraryPicker(): void {
        const webkit = (window as any).webkit;
        if (webkit?.messageHandlers?.footosPhotoPicker) {
            webkit.messageHandlers.footosPhotoPicker.postMessage({
                command: "showLimitedLibraryPicker"
            });
        }
    }

    onPermissionChange(callback: (hasFullAccess: boolean) => void): () => void {
        this.permissionCallbacks.add(callback);
        return () => this.permissionCallbacks.delete(callback);
    }

    private async waitForBridge(maxWait: number): Promise<boolean> {
        const startTime = Date.now();
        return new Promise((resolve) => {
            const check = () => {
                const webkit = (window as any).webkit;
                if (webkit?.messageHandlers?.footosPhotoPicker) {
                    resolve(true);
                    return;
                }
                if (Date.now() - startTime > maxWait) {
                    resolve(false);
                    return;
                }
                setTimeout(check, 100);
            };
            check();
        });
    }

    private async processIOSImport(paths: string[], db: string, thumb: string): Promise<void> {
        if (this.importRunning) {
            console.log("[iOS] Import already running, queuing...");
            setTimeout(() => this.processIOSImport(paths, db, thumb), 1000);
            return;
        }

        this.importRunning = true;
        console.log("[iOS] Processing", paths.length, "paths");

        this.scanningCallbacks.forEach(cb => cb(true));

        let totalSuccess = 0;
        let totalDuplicates = 0;
        let totalFailure = 0;

        for (let i = 0; i < paths.length; i++) {
            try {
                const result: any = await invoke("import_photos", {
                    rootPath: "file://" + paths[i],
                    dbPath: db,
                    thumbDir: thumb,
                });
                totalSuccess += result.success || 0;
                totalDuplicates += result.duplicates || 0;
                totalFailure += result.failure || 0;
            } catch (err) {
                console.error("[iOS] Failed to import:", paths[i], err);
                totalFailure++;
            }

            const progress: ImportProgress = {
                success: totalSuccess,
                failure: totalFailure,
                duplicates: totalDuplicates,
                current: i + 1,
                total: paths.length,
                lastPath: paths[i].split('/').pop() || paths[i],
            };
            this.progressCallbacks.forEach(cb => cb(progress));
        }

        console.log("[iOS] Import complete:", { totalSuccess, totalDuplicates, totalFailure });

        const result: ImportResult = {
            success: totalSuccess,
            failure: totalFailure,
            duplicates: totalDuplicates,
        };
        this.completeCallbacks.forEach(cb => cb(result));
        this.scanningCallbacks.forEach(cb => cb(false));
        this.importRunning = false;
    }

    async checkPermissionStatus(): Promise<PermissionStatus> {
        if (this._hasFullAccess) return 'granted';
        return 'limited';
    }

    onImportProgress(callback: (progress: ImportProgress) => void): () => void {
        this.progressCallbacks.add(callback);
        return () => this.progressCallbacks.delete(callback);
    }

    onImportComplete(callback: (result: ImportResult) => void): () => void {
        this.completeCallbacks.add(callback);
        return () => this.completeCallbacks.delete(callback);
    }

    onScanningChange(callback: (isScanning: boolean) => void): () => void {
        this.scanningCallbacks.add(callback);
        return () => this.scanningCallbacks.delete(callback);
    }

    getMapTileConfig(): MapTileConfig {
        return {
            tiles: ['https://tile.openstreetmap.org/{z}/{x}/{y}.png'],
            tileSize: 256
        };
    }
}

export function createIOSPlatformService(): IOSPlatformService {
    return new IOSPlatformServiceImpl();
}
