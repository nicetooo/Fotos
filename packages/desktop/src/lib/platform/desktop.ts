/**
 * Desktop Platform Service
 * 桌面端平台服务实现（Tauri）
 */

import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
    PlatformService,
    PlatformCapabilities,
    ImportProgress,
    ImportResult,
    PermissionStatus,
    MapTileConfig,
} from "./types";

const IMAGE_EXTENSIONS = [
    "jpg", "jpeg", "png", "heic", "heif", "webp",
    "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2",
    "dng", "raf", "orf", "rw2", "pef", "raw"
];

const VIDEO_EXTENSIONS = [
    "mp4", "mov", "m4v", "avi", "mkv", "webm",
    "flv", "wmv", "3gp", "ts"
];

const MEDIA_EXTENSIONS = [...IMAGE_EXTENSIONS, ...VIDEO_EXTENSIONS];

class DesktopPlatformService implements PlatformService {
    readonly platform = 'desktop' as const;
    readonly isMobile = false;

    readonly capabilities: PlatformCapabilities = {
        arbitraryPathAccess: true,
        nativePhotoLibrary: false,
        runtimePermissions: false,
        revealInFileManager: true,
        folderSelection: true,
        nativePhotoViewer: false,
    };

    private progressCallbacks = new Set<(p: ImportProgress) => void>();
    private completeCallbacks = new Set<(r: ImportResult) => void>();
    private scanningCallbacks = new Set<(s: boolean) => void>();
    private unlistenProgress?: UnlistenFn;
    private unlistenCancelled?: UnlistenFn;
    private unlistenReload?: UnlistenFn;

    async initialize(_dbPath: string, _thumbDir: string): Promise<void> {
        this.unlistenProgress = await listen("import-progress", (event: any) => {
            const payload = event.payload;
            const progress: ImportProgress = {
                success: payload.success,
                failure: payload.failure,
                duplicates: payload.duplicates || 0,
                current: payload.current,
                total: payload.total,
                lastPath: payload.last_path || "",
            };
            this.progressCallbacks.forEach(cb => cb(progress));
        });

        this.unlistenCancelled = await listen("import-cancelled", () => {
            this.scanningCallbacks.forEach(cb => cb(false));
            this.completeCallbacks.forEach(cb => cb({ success: 0, failure: 0, duplicates: 0 }));
        });

        this.unlistenReload = await listen("reload-photos", () => {
            // 触发重新加载，由调用方处理
        });
    }

    cleanup(): void {
        this.unlistenProgress?.();
        this.unlistenCancelled?.();
        this.unlistenReload?.();
        this.progressCallbacks.clear();
        this.completeCallbacks.clear();
        this.scanningCallbacks.clear();
    }

    async requestImport(): Promise<void> {
        const selected = await open({
            multiple: true,
            directory: false,
            filters: [{
                name: "Media Files",
                extensions: MEDIA_EXTENSIONS
            }]
        });

        if (!selected) return;

        const paths = Array.isArray(selected) ? selected : [selected];
        await this.importPaths(paths);
    }

    async requestScan(mode: 'folder' | 'file'): Promise<void> {
        const selected = await open({
            directory: mode === 'folder',
            multiple: false,
            filters: mode === 'file' ? [{
                name: "Media Files",
                extensions: MEDIA_EXTENSIONS
            }] : undefined
        });

        if (!selected) return;

        const rootPath = Array.isArray(selected) ? selected[0] : selected;
        this.scanningCallbacks.forEach(cb => cb(true));

        try {
            const result = await invoke("import_photos", {
                rootPath,
            }) as ImportResult;

            this.completeCallbacks.forEach(cb => cb(result));
        } catch (e) {
            console.error("[Desktop] Scan failed:", e);
            this.completeCallbacks.forEach(cb => cb({ success: 0, failure: 0, duplicates: 0 }));
        } finally {
            this.scanningCallbacks.forEach(cb => cb(false));
        }
    }

    private async importPaths(paths: string[]): Promise<void> {
        this.scanningCallbacks.forEach(cb => cb(true));

        let totalSuccess = 0;
        let totalDuplicates = 0;
        let totalFailure = 0;

        for (let i = 0; i < paths.length; i++) {
            const path = paths[i];
            try {
                const result: any = await invoke("import_photos", {
                    rootPath: path,
                });
                totalSuccess += result.success || 0;
                totalDuplicates += result.duplicates || 0;
                totalFailure += result.failure || 0;
            } catch {
                totalFailure++;
            }

            const progress: ImportProgress = {
                success: totalSuccess,
                failure: totalFailure,
                duplicates: totalDuplicates,
                current: i + 1,
                total: paths.length,
                lastPath: path.split('/').pop() || path,
            };
            this.progressCallbacks.forEach(cb => cb(progress));
        }

        const result: ImportResult = {
            success: totalSuccess,
            failure: totalFailure,
            duplicates: totalDuplicates,
        };
        this.completeCallbacks.forEach(cb => cb(result));
        this.scanningCallbacks.forEach(cb => cb(false));
    }

    async checkPermissionStatus(): Promise<PermissionStatus> {
        return 'granted'; // 桌面端始终有权限
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

    async openInSystemPlayer(path: string): Promise<void> {
        await invoke("open_in_system_player", { path });
    }

    getMapTileConfig(): MapTileConfig | null {
        // null = the shared default base map (shared/map-style.ts)
        return null;
    }
}

export function createDesktopPlatformService(): PlatformService {
    return new DesktopPlatformService();
}
