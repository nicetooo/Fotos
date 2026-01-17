/**
 * i18n Type Definitions
 */

export type Locale = 
    | 'en'    // English
    | 'zh'    // Chinese (中文)
    | 'ja'    // Japanese (日本語)
    | 'ko'    // Korean (한국어)
    | 'fr'    // French (Français)
    | 'de'    // German (Deutsch)
    | 'es'    // Spanish (Español)
    | 'pt'    // Portuguese (Português)
    | 'ru'    // Russian (Русский)
    | 'it'    // Italian (Italiano)
    | 'ar'    // Arabic (العربية)
    | 'nl'    // Dutch (Nederlands)
    | 'pl'    // Polish (Polski)
    | 'tr'    // Turkish (Türkçe)
    | 'vi'    // Vietnamese (Tiếng Việt)
    | 'th'    // Thai (ไทย)
    | 'id';   // Indonesian (Bahasa Indonesia)

export interface Translations {
    // App
    app: {
        name: string;
        version: string;
    };

    // Common actions
    common: {
        cancel: string;
        confirm: string;
        delete: string;
        remove: string;
        close: string;
        save: string;
        settings: string;
        import: string;
        export: string;
        loading: string;
        scanning: string;
        syncing: string;
    };

    // Navigation & UI
    nav: {
        library: string;
        map: string;
        timeline: string;
        photos: string;
        files: string;
    };

    // Import
    import: {
        importPhotos: string;
        importFolder: string;
        importFile: string;
        importing: string;
        selectPhotos: string;
        noPhotosSelected: string;
        duplicates: string;
        complete: string;
        failed: string;
        photoImportUnavailable: string;
    };

    // Library
    library: {
        noPhotos: string;
        sortBy: string;
        sortDate: string;
        sortName: string;
        sortSize: string;
        ascending: string;
        descending: string;
    };

    // Preview
    preview: {
        showInFinder: string;
        showInExplorer: string;
        previous: string;
        next: string;
        zoomIn: string;
        zoomOut: string;
        resetZoom: string;
    };

    // Photo info
    photoInfo: {
        date: string;
        dimensions: string;
        fileSize: string;
        camera: string;
        exposure: string;
        gps: string;
        raw: string;
        rawOnly: string;
        jpegPlusRaw: string;
        path: string;
    };

    // Delete
    delete: {
        title: string;
        removeFromLibrary: string;
        deleteOriginal: string;
        removeFromLibraryDesc: string;
        deleteOriginalDesc: string;
        alsoDeleteRaw: string;
        warningPermanent: string;
        file: string;
        associatedRaw: string;
    };

    // Settings
    settings: {
        title: string;
        appearance: string;
        theme: string;
        themeDark: string;
        themeLight: string;
        themeSystem: string;
        themeAuto: string;
        language: string;
        storage: string;
        database: string;
        thumbnails: string;
        cache: string;
        clearCache: string;
        clearAllData: string;
        clearCacheConfirm: string;
        about: string;
        techStack: string;
    };

    // Permissions (mobile)
    permissions: {
        photoLibraryAccess: string;
        photoLibraryDenied: string;
        photoLibraryLimited: string;
        goToSettings: string;
        selectMorePhotos: string;
        selectFromAlbum: string;
        limitedAccessTitle: string;
        limitedAccessMessage: string;
        deniedAccessTitle: string;
        deniedAccessMessage: string;
    };

    // Errors
    errors: {
        initFailed: string;
        importFailed: string;
        deleteFailed: string;
        loadFailed: string;
        openFailed: string;
        clearFailed: string;
    };
}

export interface I18nContext {
    locale: Locale;
    t: Translations;
    setLocale: (locale: Locale) => void;
}
