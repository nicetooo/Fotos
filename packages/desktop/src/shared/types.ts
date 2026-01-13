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
}

export interface PhotoInfo {
    id: PhotoId;
    path: string;
    hash: string;
    metadata: PhotoMetadata;
    thumb_path?: string;
    file_size: number;
    created_at?: number;
    modified_at?: number;
    // For grouped RAW+JPEG pairs
    rawPath?: string;
    hasRaw?: boolean;
    isRawOnly?: boolean;
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
