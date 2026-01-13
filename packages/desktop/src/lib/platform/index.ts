/**
 * Platform Service Module
 * 平台服务模块导出
 */

// Types
export type {
    PlatformCapabilities,
    ImportProgress,
    ImportResult,
    PermissionStatus,
    MapTileConfig,
    PlatformService,
    IOSPlatformService,
} from "./types";

// Context utilities
export {
    detectAndCreatePlatformService,
    setPlatformService,
    getPlatformService,
    getIOSPlatformService,
    isIOSPlatform,
} from "./context";

// Platform implementations (for direct usage if needed)
export { createDesktopPlatformService } from "./desktop";
export { createIOSPlatformService } from "./ios";
export { createAndroidPlatformService } from "./android";
