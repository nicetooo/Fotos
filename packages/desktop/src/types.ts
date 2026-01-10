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
}
