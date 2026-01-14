/**
 * Platform Service Context
 * Svelte Context 工具和平台服务工厂
 */

import { getContext, setContext } from "svelte";
import { platform } from "@tauri-apps/plugin-os";
import type { PlatformService, IOSPlatformService, AndroidPlatformService } from "./types";
import { createDesktopPlatformService } from "./desktop";
import { createIOSPlatformService } from "./ios";
import { createAndroidPlatformService } from "./android";

const PLATFORM_SERVICE_KEY = Symbol("platform-service");

/** 检测当前平台并创建对应的服务实例 */
export async function detectAndCreatePlatformService(): Promise<PlatformService> {
    const platformName = await platform();
    console.log("[Platform] Detected platform:", platformName);

    switch (platformName) {
        case "ios":
            return createIOSPlatformService();
        case "android":
            return createAndroidPlatformService();
        default:
            // macOS, Windows, Linux 都使用桌面服务
            return createDesktopPlatformService();
    }
}

/** 设置平台服务到 Svelte Context */
export function setPlatformService(service: PlatformService): void {
    setContext(PLATFORM_SERVICE_KEY, service);
}

/** 从 Svelte Context 获取平台服务 */
export function getPlatformService(): PlatformService {
    const service = getContext<PlatformService>(PLATFORM_SERVICE_KEY);
    if (!service) {
        throw new Error("Platform service not found in context. Did you call setPlatformService?");
    }
    return service;
}

/** 获取 iOS 特定服务（仅在 iOS 平台可用）*/
export function getIOSPlatformService(): IOSPlatformService {
    const service = getPlatformService();
    if (service.platform !== 'ios') {
        throw new Error("iOS platform service only available on iOS");
    }
    return service as IOSPlatformService;
}

/** 检查是否是 iOS 平台 */
export function isIOSPlatform(service: PlatformService): service is IOSPlatformService {
    return service.platform === 'ios';
}

/** 获取 Android 特定服务（仅在 Android 平台可用）*/
export function getAndroidPlatformService(): AndroidPlatformService {
    const service = getPlatformService();
    if (service.platform !== 'android') {
        throw new Error("Android platform service only available on Android");
    }
    return service as AndroidPlatformService;
}

/** 检查是否是 Android 平台 */
export function isAndroidPlatform(service: PlatformService): service is AndroidPlatformService {
    return service.platform === 'android';
}

/** 检查是否是移动端平台 */
export function isMobilePlatform(service: PlatformService): boolean {
    return service.isMobile;
}
