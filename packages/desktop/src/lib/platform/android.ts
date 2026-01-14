/**
 * Android Platform Service
 * Android 平台服务实现（WebView 桥接）
 *
 * Communication patterns:
 * - JS -> Kotlin: window.AndroidWebViewInterface.postMessage(JSON)
 * - Kotlin -> JS: CustomEvent dispatch via webView.evaluateJavascript()
 * - Kotlin -> JS: Global function call (window.__handleAndroidImportPaths)
 */

import { invoke } from "@tauri-apps/api/core";
import type {
    PlatformService,
    PlatformCapabilities,
    ImportProgress,
    ImportResult,
    PermissionStatus,
    MapTileConfig,
} from "./types";

// Extend Window interface for Android bridge
declare global {
    interface Window {
        AndroidWebViewInterface?: {
            postMessage: (message: string) => void;
        };
        __ANDROID_BRIDGE_READY__?: boolean;
        __handleAndroidImportPaths?: (paths: string[], dbPath: string, thumbDir: string) => void;
    }
}

type EventHandler = { event: string; handler: EventListener };

/**
 * Android Platform Service implementation
 * Mirrors iOS implementation pattern with WebKit MessageHandler replaced by
 * Android's @JavascriptInterface
 */
class AndroidPlatformService implements PlatformService {
    readonly platform = 'android' as const;
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
    private permissionCallbacks = new Set<(hasAccess: boolean) => void>();
    private eventListeners: EventHandler[] = [];
    private dbPath = '';
    private thumbDir = '';
    private _hasMediaAccess = false;
    private _bridgeReady = false;
    private importRunning = false;

    /**
     * Check if user has granted media access permission
     */
    get hasMediaAccess(): boolean {
        return this._hasMediaAccess;
    }

    async initialize(dbPath: string, thumbDir: string): Promise<void> {
        this.dbPath = dbPath;
        this.thumbDir = thumbDir;

        // Android import progress event
        this.addListener('android-import-progress', (e: CustomEvent) => {
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

        // Android import complete event
        this.addListener('android-import-complete', (e: CustomEvent) => {
            const detail = e.detail;
            console.log("[Android] Import complete:", detail);
            this.scanningCallbacks.forEach(cb => cb(false));
            this.completeCallbacks.forEach(cb => cb({
                success: detail.success || 0,
                failure: detail.failure || 0,
                duplicates: detail.duplicates || 0,
            }));
            this.importRunning = false;
        });

        // Android import started event
        this.addListener('android-import-started', () => {
            console.log("[Android] Import started");
            this.scanningCallbacks.forEach(cb => cb(true));
            this.importRunning = true;
        });

        // Android permission granted event
        this.addListener('android-permission-granted', (e: CustomEvent) => {
            const detail = e.detail;
            console.log("[Android] Permission granted:", detail.type);
            this._hasMediaAccess = true;
            this.permissionCallbacks.forEach(cb => cb(true));
        });

        // Android permission denied event
        this.addListener('android-permission-denied', (e: CustomEvent) => {
            const detail = e.detail;
            console.log("[Android] Permission denied:", detail.message);
            this._hasMediaAccess = false;
            this.permissionCallbacks.forEach(cb => cb(false));
            this.scanningCallbacks.forEach(cb => cb(false));
        });

        // Android permission status event
        this.addListener('android-permission-status', (e: CustomEvent) => {
            const hasAccess = e.detail.hasAccess;
            console.log("[Android] Permission status:", hasAccess);
            this._hasMediaAccess = hasAccess;
            this.permissionCallbacks.forEach(cb => cb(hasAccess));
        });

        // Android error event
        this.addListener('android-error', (e: CustomEvent) => {
            console.error("[Android] Error:", e.detail.message);
            this.scanningCallbacks.forEach(cb => cb(false));
        });

        // Android bridge ready event
        this.addListener('android-bridge-ready', () => {
            console.log("[Android] Bridge is ready");
            this._bridgeReady = true;
        });

        // Check if bridge was already ready
        if (window.__ANDROID_BRIDGE_READY__) {
            console.log("[Android] Bridge was already ready");
            this._bridgeReady = true;
        }

        // Expose global function for Kotlin to call
        window.__handleAndroidImportPaths = (paths: string[], db: string, thumb: string) => {
            console.log("[Android] Received paths via global function:", paths?.length);
            if (!paths || paths.length === 0) {
                this.scanningCallbacks.forEach(cb => cb(false));
                return;
            }
            this.processAndroidImport(paths, db || this.dbPath, thumb || this.thumbDir);
        };

        console.log("[Android] Platform service initialized");
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
        delete window.__handleAndroidImportPaths;
        this.progressCallbacks.clear();
        this.completeCallbacks.clear();
        this.scanningCallbacks.clear();
        this.permissionCallbacks.clear();
    }

    /**
     * Request photo import via Photo Picker or GetContent
     */
    async requestImport(): Promise<void> {
        console.log("[Android] Requesting import, bridge ready:", this._bridgeReady);

        const bridgeAvailable = window.AndroidWebViewInterface || window.__ANDROID_BRIDGE_READY__;

        if (bridgeAvailable) {
            console.log("[Android] Using AndroidWebViewInterface");
            this.postMessage({
                command: "requestImport",
                dbPath: this.dbPath,
                thumbDir: this.thumbDir
            });
            return;
        }

        // Bridge not ready, wait and retry
        console.log("[Android] Bridge not ready, waiting...");
        this.scanningCallbacks.forEach(cb => cb(true));

        const ready = await this.waitForBridge(3000);
        if (ready) {
            console.log("[Android] Bridge became ready, proceeding");
            this.postMessage({
                command: "requestImport",
                dbPath: this.dbPath,
                thumbDir: this.thumbDir
            });
        } else {
            console.error("[Android] Bridge not available after timeout");
            this.scanningCallbacks.forEach(cb => cb(false));
        }
    }

    /**
     * Sync all photos from MediaStore (requires permission)
     */
    async syncPhotosIfHasAccess(): Promise<void> {
        console.log("[Android] Checking for permission to auto-sync...");

        const ready = await this.waitForBridge(3000);
        if (ready) {
            this.postMessage({
                command: "syncMediaStore",
                dbPath: this.dbPath,
                thumbDir: this.thumbDir
            });
        } else {
            console.log("[Android] Bridge not ready for auto-sync");
        }
    }

    /**
     * Request media access permission
     */
    async requestPermission(): Promise<void> {
        this.postMessage({
            command: "requestPermission",
            dbPath: this.dbPath,
            thumbDir: this.thumbDir
        });
    }

    /**
     * Subscribe to permission changes
     */
    onPermissionChange(callback: (hasAccess: boolean) => void): () => void {
        this.permissionCallbacks.add(callback);
        return () => this.permissionCallbacks.delete(callback);
    }

    /**
     * Cancel current import operation
     */
    cancelImport(): void {
        this.postMessage({ command: "cancelImport" });
    }

    private postMessage(data: Record<string, unknown>): void {
        const message = JSON.stringify(data);
        if (window.AndroidWebViewInterface) {
            window.AndroidWebViewInterface.postMessage(message);
        } else {
            console.warn("[Android] AndroidWebViewInterface not available");
        }
    }

    private async waitForBridge(maxWait: number): Promise<boolean> {
        const startTime = Date.now();
        return new Promise((resolve) => {
            const check = () => {
                if (window.AndroidWebViewInterface || window.__ANDROID_BRIDGE_READY__) {
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

    /**
     * Process imported paths (called from global function or directly)
     */
    private async processAndroidImport(paths: string[], db: string, thumb: string): Promise<void> {
        if (this.importRunning) {
            console.log("[Android] Import already running, queuing...");
            setTimeout(() => this.processAndroidImport(paths, db, thumb), 1000);
            return;
        }

        this.importRunning = true;
        console.log("[Android] Processing", paths.length, "paths");

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
                console.error("[Android] Failed to import:", paths[i], err);
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

        console.log("[Android] Import complete:", { totalSuccess, totalDuplicates, totalFailure });

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
        if (this._hasMediaAccess) return 'granted';
        return 'denied';
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

    getMapTileConfig(): MapTileConfig | null {
        // Return null to use default CARTO tiles with dark/light theme support
        return null;
    }
}

/**
 * Extended Android Platform Service with MediaStore sync support
 */
export interface AndroidPlatformServiceExtended extends PlatformService {
    readonly hasMediaAccess: boolean;
    syncPhotosIfHasAccess(): Promise<void>;
    requestPermission(): Promise<void>;
    cancelImport(): void;
    onPermissionChange(callback: (hasAccess: boolean) => void): () => void;
}

export function createAndroidPlatformService(): AndroidPlatformServiceExtended {
    return new AndroidPlatformService();
}
