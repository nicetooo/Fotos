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
    /** 地图由平台自己的地图视图绘制，垫在透明的 WebView 下面 (iOS: true)。为 false 时用 MapLibre */
    nativeMap: boolean;
    /** 相册网格的长按菜单由系统原生菜单承担 (iOS: true)。为 false 时网格自己画菜单 */
    nativeContextMenu: boolean;
    /** 有原生照片查看器 (iOS、Android: true)。为 true 时用 openNativeViewer，否则用 Web 查看器 */
    nativePhotoViewer: boolean;
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

    /**
     * Better thumbnails for the album grid, made on demand for the given item
     * paths. `onReady` gets a map of item path -> image file as they arrive.
     * Each call replaces the previous request.
     */
    requestGridThumbs?(paths: string[], onReady: (thumbs: Record<string, string>) => void): void;

    /**
     * Show the system's own menu for a library item at a point on the page
     * (CSS pixels). Copy and share are handled by the platform; `onLocate`
     * runs when the user picks "show on map".
     */
    showItemMenu?(
        request: { x: number; y: number; path: string; dark: boolean; labels: { copy: string; share: string; locate: string } },
        onLocate: () => void,
    ): void;

    /** Hand an item (its original) to the system share sheet. */
    shareItem?(path: string): void;
    /** Put an item (its original) on the clipboard. */
    copyItem?(path: string): void;

    /** 重新扫描记住的文件夹（桌面端）；返回库是否有变化 */
    rescanLibrary?(): Promise<boolean>;

    /** 订阅导入进度 */
    onImportProgress(callback: (progress: ImportProgress) => void): () => void;

    /** 订阅导入完成 */
    onImportComplete(callback: (result: ImportResult) => void): () => void;

    /** 订阅扫描状态变化 */
    onScanningChange(callback: (isScanning: boolean) => void): () => void;

    /** 获取地图瓦片配置，返回 null 则使用默认底图（shared/map-style.ts） */
    getMapTileConfig(): MapTileConfig | null;

    /** 用系统播放器打开文件 */
    openInSystemPlayer?(path: string): Promise<void>;
}

/** 原生查看器的一项 */
export interface NativeViewerItem {
    /** `ios-photo://<localIdentifier>` 或本地文件路径 */
    path: string;
    thumb: string;
    media_type: string;
    date_taken: string;
}

/** 有原生查看器的平台实现这个接口（`capabilities.nativePhotoViewer` 为 true 时） */
export interface NativeViewerHost {
    /**
     * 用原生查看器打开一组照片（时间顺序），index 为起始位置。
     * 返回 false 表示原生桥不可用，调用方应退回 Web 查看器。
     */
    openNativeViewer(items: NativeViewerItem[], index: number): boolean;
}

/** 平台服务是否带原生查看器 */
export function nativeViewerOf(service: PlatformService): NativeViewerHost | null {
    const host = service as PlatformService & Partial<NativeViewerHost>;
    return service.capabilities.nativePhotoViewer && typeof host.openNativeViewer === "function"
        ? (host as NativeViewerHost)
        : null;
}

/** iOS 扩展接口 */
export interface IOSPlatformService extends PlatformService, NativeViewerHost {
    /** 是否有完整相册访问权限 */
    readonly hasFullPhotoAccess: boolean;

    /** 记录已入库的 PhotoKit localIdentifier，供增量同步跳过 */
    setKnownPhotoIdentifiers(ids: string[]): void;

    /** 如果有完整权限则增量同步新照片 */
    syncPhotosIfFullAccess(): Promise<void>;

    /** 显示有限权限选择器 */
    showLimitedLibraryPicker(): void;

    /** 订阅权限状态变化 */
    onPermissionChange(callback: (hasFullAccess: boolean) => void): () => void;

    /** 点开大图时按需导出原片，返回本地文件路径 */
    ensureOriginal(path: string): Promise<string>;

    /** 视频/实况封面：屏幕尺寸静帧，比 256 缩略图清晰，也比整段视频快 */
    ensureViewerStill(path: string): Promise<string | null>;

    /**
     * 告诉原生侧相册网格里现在有哪些条目，以及长按菜单的文案。
     * 网格格子是 `footos-album://item/<index>` 链接，长按时系统菜单按下标取条目。
     */
    setAlbumMenuContext(items: NativeViewerItem[], titles: { copy: string; share: string; locate: string }): void;

    /** 给原生地图发一条命令（mapAttach / mapSetFeatures / …，见 FootosMapView.h） */
    nativeMapCommand(name: string, payload?: Record<string, unknown>): void;

    /** 没有权限时用户点了导入：原生提示框的文案 */
    setAccessDeniedTexts(texts: { title: string; message: string; settings: string; cancel: string }): void;

    /** 原生同步发现库里有照片已不可见并清掉了对应的行 */
    onLibraryChanged(callback: () => void): () => void;

    /** 应用数据被清空后：丢掉内存里指向已删除文件的缓存，并清原生导出的文件 */
    resetAfterClear(): void;

    /** 实况配对视频，长按时播放 */
    ensureLiveVideo(path: string): Promise<string | null>;
}

/** Android 扩展接口 */
export interface AndroidPlatformService extends PlatformService, NativeViewerHost {
    /** 是否有媒体访问权限 */
    readonly hasMediaAccess: boolean;

    /** 如果有权限则同步 MediaStore 照片 */
    syncPhotosIfHasAccess(prompt?: boolean): Promise<void>;

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
