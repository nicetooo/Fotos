/**
 * Shared Types
 * 共享类型定义
 */

export interface PhotoId {
    id: number;
}

export interface PhotoMetadata {
    width: number;
    height: number;
    date_taken?: string;
    iso?: number;
    f_number?: number;
    exposure_time?: string;
    make?: string;
    model?: string;
    lat?: number;
    lon?: number;
    duration_ms?: number;
    video_codec?: string;
    audio_codec?: string;
}

export interface PhotoInfo {
    id: PhotoId;
    path: string;
    hash: string;
    metadata: PhotoMetadata;
    thumb_path?: string;
    file_size: number;
    // For grouped RAW+JPEG pairs
    rawPath?: string;
    hasRaw?: boolean;
    isRawOnly?: boolean;
    // Video / Live Photo support
    media_type?: 'photo' | 'video' | 'live_photo';
    video_path?: string;
}

/**
 * Lightweight photo preview for map markers and timeline.
 * Contains only the fields needed for initial UI rendering.
 * Zero file I/O — all data comes from the database.
 */
export interface PhotoPreview {
    id: number;
    path: string;
    thumb_path?: string;
    lat?: number;
    lon?: number;
    date_taken?: string;
    file_size: number;
    media_type?: string;
    duration_ms?: number;
}

/** 排序字段 */
export type SortBy = "name" | "date" | "size" | "dimensions";

/** 排序方向 */
export type SortOrder = "asc" | "desc";

/** 主题 */
export type Theme = "dark" | "light" | "system";

/** 导入状态 */
export interface ImportStatus {
    success: number;
    failure: number;
    duplicates: number;
    current: number;
    total: number;
    lastPath: string;
}
