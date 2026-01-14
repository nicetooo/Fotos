/**
 * Platform Service Types
 * 平台服务抽象层类型定义
 */

/** 平台能力标志 */
export interface PlatformCapabilities {
    /** 可以访问任意文件路径 (Desktop: true, Mobile: false) */
    arbitraryPathAccess: boolean;
    /** 有原生相册集成 (Desktop: false, Mobile: true) */
    nativePhotoLibrary: boolean;
    /** 需要运行时权限 (Desktop: false, Mobile: true) */
    runtimePermissions: boolean;
    /** 可以在文件管理器中显示 (Desktop: true, Mobile: false) */
    revealInFileManager: boolean;
    /** 支持文件夹选择 (Desktop: true, Mobile: false) */
    folderSelection: boolean;
}

/** 导入结果 */
export interface ImportResult {
    success: number;
    failure: number;
    duplicates: number;
}

/** 导入进度 */
export interface ImportProgress {
    current: number;
    total: number;
    success: number;
    failure: number;
    duplicates: number;
    lastPath: string;
}

/** 权限状态 */
export type PermissionStatus = 'granted' | 'denied' | 'limited' | 'unknown';

/** 地图瓦片配置 */
export interface MapTileConfig {
    tiles: string[];
    tileSize: number;
}

/** 平台服务接口 */
export interface PlatformService {
    /** 平台标识 */
    readonly platform: 'desktop' | 'ios' | 'android';

    /** 平台能力 */
    readonly capabilities: PlatformCapabilities;

    /** 是否是移动端 */
    readonly isMobile: boolean;

    /** 初始化平台特定配置（在 mount 时调用一次）*/
    initialize(dbPath: string, thumbDir: string): Promise<void>;

    /** 清理资源 */
    cleanup(): void;

    /** 请求导入照片 */
    requestImport(): Promise<void>;

    /** 请求扫描文件夹/文件（桌面端） */
    requestScan?(mode: 'folder' | 'file'): Promise<void>;

    /** 检查权限状态（移动端）*/
    checkPermissionStatus(): Promise<PermissionStatus>;

    /** 订阅导入进度 */
    onImportProgress(callback: (progress: ImportProgress) => void): () => void;

    /** 订阅导入完成 */
    onImportComplete(callback: (result: ImportResult) => void): () => void;

    /** 订阅扫描状态变化 */
    onScanningChange(callback: (isScanning: boolean) => void): () => void;

    /** 获取地图瓦片配置，返回 null 则使用默认 CARTO 瓦片 */
    getMapTileConfig(): MapTileConfig | null;
}

/** iOS 扩展接口 */
export interface IOSPlatformService extends PlatformService {
    /** 是否有完整相册访问权限 */
    readonly hasFullPhotoAccess: boolean;

    /** 如果有完整权限则同步照片 */
    syncPhotosIfFullAccess(): Promise<void>;

    /** 显示有限权限选择器 */
    showLimitedLibraryPicker(): void;

    /** 订阅权限状态变化 */
    onPermissionChange(callback: (hasFullAccess: boolean) => void): () => void;
}

/** Android 扩展接口 */
export interface AndroidPlatformService extends PlatformService {
    /** 是否有媒体访问权限 */
    readonly hasMediaAccess: boolean;

    /** 如果有权限则同步 MediaStore 照片 */
    syncPhotosIfHasAccess(): Promise<void>;

    /** 请求媒体访问权限 */
    requestPermission(): Promise<void>;

    /** 取消当前导入操作 */
    cancelImport(): void;

    /** 订阅权限状态变化 */
    onPermissionChange(callback: (hasAccess: boolean) => void): () => void;
}

// ==================== Type Guards ====================

/** 检查是否是 iOS 平台服务 */
export function isIOSPlatform(service: PlatformService): service is IOSPlatformService {
    return service.platform === 'ios';
}

/** 检查是否是 Android 平台服务 */
export function isAndroidPlatform(service: PlatformService): service is AndroidPlatformService {
    return service.platform === 'android';
}

/** 检查是否是移动端平台 */
export function isMobilePlatform(service: PlatformService): service is IOSPlatformService | AndroidPlatformService {
    return service.isMobile;
}
