/**
 * Sorting Utilities
 * 照片排序逻辑
 */

import type { PhotoInfo, SortBy, SortOrder } from "../types";
import { getFileName } from "./photo-grouping";

/**
 * 排序照片
 */
export function sortPhotos(
    photos: PhotoInfo[],
    sortBy: SortBy,
    sortOrder: SortOrder
): PhotoInfo[] {
    const sorted = [...photos];

    sorted.sort((a, b) => {
        let comparison = 0;

        switch (sortBy) {
            case "name":
                const nameA = getFileName(a.path);
                const nameB = getFileName(b.path);
                comparison = nameA.localeCompare(nameB);
                break;

            case "date":
                const dateA = a.metadata.date_taken || "";
                const dateB = b.metadata.date_taken || "";
                comparison = dateA.localeCompare(dateB);
                break;

            case "size":
            case "dimensions":
                const pixelsA = a.metadata.width * a.metadata.height;
                const pixelsB = b.metadata.width * b.metadata.height;
                comparison = pixelsA - pixelsB;
                break;
        }

        return sortOrder === "asc" ? comparison : -comparison;
    });

    return sorted;
}

/**
 * 按日期过滤照片
 */
export function filterPhotosByDateRange(
    photos: PhotoInfo[],
    startDate?: Date,
    endDate?: Date
): PhotoInfo[] {
    if (!startDate && !endDate) return photos;

    return photos.filter(photo => {
        if (!photo.metadata.date_taken) return false;

        const photoDate = new Date(photo.metadata.date_taken.replace(/^(\d{4}):(\d{2}):(\d{2})/, '$1-$2-$3'));
        if (isNaN(photoDate.getTime())) return false;

        if (startDate && photoDate < startDate) return false;
        if (endDate && photoDate > endDate) return false;

        return true;
    });
}

/**
 * 按位置过滤有地理信息的照片
 */
export function filterPhotosWithLocation(photos: PhotoInfo[]): PhotoInfo[] {
    return photos.filter(photo =>
        photo.metadata.lat != null &&
        photo.metadata.lon != null &&
        !isNaN(photo.metadata.lat) &&
        !isNaN(photo.metadata.lon)
    );
}
