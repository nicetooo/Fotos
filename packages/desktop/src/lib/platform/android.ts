/**
 * Android Platform Service
 * Android 平台服务实现（预留）
 */

import type {
    PlatformService,
    PlatformCapabilities,
    ImportProgress,
    ImportResult,
    PermissionStatus,
    MapTileConfig,
} from "./types";

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

    private scanningCallbacks = new Set<(s: boolean) => void>();

    async initialize(_dbPath: string, _thumbDir: string): Promise<void> {
        // TODO: Android JNI 桥接初始化
        console.log("[Android] Platform service initialized (stub)");
    }

    cleanup(): void {
        this.scanningCallbacks.clear();
    }

    async requestImport(): Promise<void> {
        console.warn("[Android] Import not implemented");
        // TODO: 实现 Android 导入
    }

    async checkPermissionStatus(): Promise<PermissionStatus> {
        return 'unknown';
    }

    onImportProgress(_callback: (progress: ImportProgress) => void): () => void {
        return () => {};
    }

    onImportComplete(_callback: (result: ImportResult) => void): () => void {
        return () => {};
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

export function createAndroidPlatformService(): PlatformService {
    return new AndroidPlatformService();
}
