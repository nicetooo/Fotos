/**
 * Photo Grouping Utilities
 * RAW/JPEG 分组逻辑
 */

import type { PhotoInfo } from "../types";

/** RAW 文件扩展名 */
const RAW_EXTENSIONS = new Set([
    "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2",
    "dng", "raf", "orf", "rw2", "pef", "raw"
]);

/** JPEG 文件扩展名 */
const JPEG_EXTENSIONS = new Set(["jpg", "jpeg"]);

/** 检查是否是 RAW 文件 */
export function isRawFile(path: string): boolean {
    const ext = path.split(".").pop()?.toLowerCase() || "";
    return RAW_EXTENSIONS.has(ext);
}

/** 检查是否是 JPEG 文件 */
export function isJpegFile(path: string): boolean {
    const ext = path.split(".").pop()?.toLowerCase() || "";
    return JPEG_EXTENSIONS.has(ext);
}

/** 获取文件基础名（不含扩展名） */
export function getBaseName(path: string): string {
    const fileName = path.split("/").pop() || "";
    const lastDot = fileName.lastIndexOf(".");
    return lastDot > 0 ? fileName.substring(0, lastDot).toLowerCase() : fileName.toLowerCase();
}

/** 获取文件名（含扩展名） */
export function getFileName(path: string): string {
    return path.split("/").pop() || path;
}

/**
 * 分组 RAW+JPEG 配对
 * - JPEG 有配对 RAW 时显示 RAW 徽章
 * - 只有 RAW 时标记为 isRawOnly
 */
export function groupPhotos(photos: PhotoInfo[]): PhotoInfo[] {
    // 按基础名分组
    const byBaseName = new Map<string, { jpegs: PhotoInfo[]; raws: PhotoInfo[] }>();

    for (const photo of photos) {
        const baseName = getBaseName(photo.path);
        if (!byBaseName.has(baseName)) {
            byBaseName.set(baseName, { jpegs: [], raws: [] });
        }
        const group = byBaseName.get(baseName)!;
        if (isRawFile(photo.path)) {
            group.raws.push(photo);
        } else if (isJpegFile(photo.path)) {
            group.jpegs.push(photo);
        } else {
            // 其他格式（PNG 等）当作 JPEG 处理
            group.jpegs.push(photo);
        }
    }

    // 构建结果
    const result: PhotoInfo[] = [];
    for (const [_, group] of byBaseName) {
        if (group.jpegs.length > 0) {
            // 有 JPEG - 显示 JPEG，如果有 RAW 则添加徽章
            for (const jpeg of group.jpegs) {
                if (group.raws.length > 0) {
                    result.push({
                        ...jpeg,
                        hasRaw: true,
                        rawPath: group.raws[0].path,
                    });
                } else {
                    result.push(jpeg);
                }
            }
        } else {
            // 只有 RAW - 标记为独立 RAW
            for (const raw of group.raws) {
                result.push({
                    ...raw,
                    isRawOnly: true,
                });
            }
        }
    }

    return result;
}

/**
 * 获取配对 RAW 文件的大小
 */
export function getRawFileSize(photo: PhotoInfo, allPhotos: PhotoInfo[]): number {
    if (!photo.hasRaw || !photo.rawPath) return 0;
    const rawPhoto = allPhotos.find(p => p.path === photo.rawPath);
    return rawPhoto?.file_size || 0;
}
