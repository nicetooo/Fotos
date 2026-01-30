import type { Translations } from './types';

export const en: Translations = {
    app: {
        name: 'footos',
        version: 'v',
    },

    common: {
        cancel: 'Cancel',
        confirm: 'Confirm',
        delete: 'Delete',
        remove: 'Remove',
        close: 'Close',
        save: 'Save',
        settings: 'Settings',
        import: 'Import',
        export: 'Export',
        loading: 'Loading...',
        scanning: 'Scanning...',
        syncing: 'Syncing...',
    },

    nav: {
        library: 'Library',
        map: 'Map',
        timeline: 'Timeline',
        photos: 'Photos',
        files: 'files',
    },

    import: {
        importPhotos: 'Import photos',
        importFolder: 'Import Folder',
        importFile: 'Import File',
        importing: 'Importing...',
        selectPhotos: 'Select Photos',
        noPhotosSelected: 'No photos selected',
        duplicates: 'dup',
        complete: 'Import complete',
        failed: 'Import failed',
        photoImportUnavailable: 'Photo import not available. Please restart the app.',
    },

    library: {
        noPhotos: 'No photos',
        sortBy: 'Sort by',
        sortDate: 'Date',
        sortName: 'Name',
        sortSize: 'Size',
        ascending: 'Ascending',
        descending: 'Descending',
        filterAll: 'All',
        filterPhotos: 'Photos',
        filterVideos: 'Videos',
        filterLivePhotos: 'Live',
    },

    preview: {
        showInFinder: 'Show in Finder',
        showInExplorer: 'Show in Explorer',
        previous: 'Previous',
        next: 'Next',
        zoomIn: 'Zoom in',
        zoomOut: 'Zoom out',
        resetZoom: 'Reset zoom',
    },

    photoInfo: {
        date: 'Date',
        dimensions: 'Dimensions',
        fileSize: 'File Size',
        camera: 'Camera',
        exposure: 'Exposure',
        gps: 'GPS',
        raw: 'RAW',
        rawOnly: 'RAW Only',
        jpegPlusRaw: 'JPEG + RAW',
        path: 'Path',
    },

    delete: {
        title: 'Delete',
        removeFromLibrary: 'Remove from Library',
        deleteOriginal: 'Delete Original File',
        removeFromLibraryDesc: 'Original file will be kept',
        deleteOriginalDesc: 'This will permanently delete the original file',
        alsoDeleteRaw: 'Also delete RAW file',
        warningPermanent: 'Warning: This action cannot be undone. The original file will be permanently deleted from disk.',
        file: 'File',
        associatedRaw: 'This photo has an associated RAW file:',
    },

    settings: {
        title: 'Settings',
        appearance: 'Appearance',
        theme: 'Theme',
        themeDark: 'Dark',
        themeLight: 'Light',
        themeSystem: 'System',
        themeAuto: 'Auto',
        language: 'Language',
        storage: 'Storage',
        database: 'Database',
        thumbnails: 'Thumbnails',
        cache: 'Cache',
        clearCache: 'Clear Cache',
        clearAllData: 'Clear All Data',
        clearCacheConfirm: 'This will delete all imported photos and thumbnails. Map cache will be preserved.\n\nContinue?',
        about: 'About',
        techStack: 'Tauri + Svelte + Rust',
    },

    permissions: {
        photoLibraryAccess: 'Photo Library Access',
        photoLibraryDenied: 'Photo library access denied',
        photoLibraryLimited: 'Limited photo access',
        goToSettings: 'Go to Settings',
        selectMorePhotos: 'Select More Photos',
        selectFromAlbum: 'Select from Album',
        limitedAccessTitle: 'Limited Photo Access',
        limitedAccessMessage: 'You have only authorized access to some photos. You can select more photos or allow access to all photos in Settings.',
        deniedAccessTitle: 'Cannot Access Photos',
        deniedAccessMessage: 'Please allow footos to access your photo library in Settings.',
    },

    map: {
        showAllPhotos: 'Show all photos',
        boxSelect: 'Box select photos',
        noGeotaggedPhotos: 'No Geotagged Photos',
        noGeotaggedPhotosDesc: 'None of your imported photos have GPS data.',
    },

    imageError: {
        unableToLoad: 'Unable to load image',
        rawPreviewFailed: 'Failed to extract preview from RAW file',
        fileMissing: 'File may have been moved or the drive disconnected',
        extractingRaw: 'Extracting RAW preview...',
        showingThumbnail: 'Showing thumbnail (full preview unavailable)',
    },

    timeline: {
        all: 'All',
        reset: 'Reset',
        range: 'Range',
    },

    errors: {
        initFailed: 'Failed to initialize',
        importFailed: 'Import failed',
        deleteFailed: 'Delete failed',
        loadFailed: 'Failed to load photos',
        openFailed: 'Failed to open',
        clearFailed: 'Failed to clear data',
    },
};
