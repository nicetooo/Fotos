/**
 * Format Utilities
 * 格式化工具函数
 */

/** 格式化文件大小 */
export function formatFileSize(bytes: number): string {
    if (bytes === 0) return "0 B";
    const units = ["B", "KB", "MB", "GB"];
    const k = 1024;
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    const size = bytes / Math.pow(k, i);
    return `${size.toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}

/** 格式化日期 */
export function formatDate(date: Date): string {
    return date.toLocaleDateString('en-US', {
        month: 'short',
        day: 'numeric',
        year: 'numeric'
    });
}

/** 格式化日期时间 */
export function formatDateTime(date: Date): string {
    return `${formatDate(date)} ${date.toLocaleTimeString('en-US', {
        hour: '2-digit',
        minute: '2-digit'
    })}`;
}

/** 格式化 EXIF 日期字符串 */
export function formatExifDate(dateStr: string | undefined): string {
    if (!dateStr) return "Unknown";
    try {
        // EXIF format: "2024:01:15 10:30:00" or ISO format
        const normalized = dateStr.replace(/^(\d{4}):(\d{2}):(\d{2})/, '$1-$2-$3');
        const date = new Date(normalized);
        if (isNaN(date.getTime())) return dateStr;
        return formatDateTime(date);
    } catch {
        return dateStr;
    }
}

/** 格式化相机信息 */
export function formatCameraInfo(make?: string, model?: string): string {
    if (!make && !model) return "Unknown";
    if (make && model) {
        // 避免重复（如 "Canon Canon EOS R5"）
        if (model.toLowerCase().startsWith(make.toLowerCase())) {
            return model;
        }
        return `${make} ${model}`;
    }
    return make || model || "Unknown";
}

/** 格式化曝光信息 */
export function formatExposure(exposureTime?: string, fNumber?: number, iso?: number): string {
    const parts: string[] = [];
    if (exposureTime) parts.push(exposureTime);
    if (fNumber) parts.push(`f/${fNumber}`);
    if (iso) parts.push(`ISO ${iso}`);
    return parts.length > 0 ? parts.join(" · ") : "";
}

/** 格式化图片尺寸 */
export function formatDimensions(width: number, height: number): string {
    const megapixels = (width * height) / 1_000_000;
    return `${width} × ${height} (${megapixels.toFixed(1)} MP)`;
}
