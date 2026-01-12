//! JNI bridge for Android native methods
//!
//! This module provides the JNI interface that Kotlin/Java code calls
//! to interact with the Rust photo processing library.

use jni::JNIEnv;
use jni::objects::{JClass, JString, JByteArray};
use jni::sys::{jint, jboolean, jlong};

use crate::AndroidPhotoService;
use fotos_shared::PhotoService;
use std::sync::OnceLock;

static PHOTO_SERVICE: OnceLock<AndroidPhotoService> = OnceLock::new();

fn get_photo_service() -> &'static AndroidPhotoService {
    PHOTO_SERVICE.get_or_init(AndroidPhotoService::new)
}

/// Initialize the Rust library
/// Called from Kotlin: FotosNative.init(filesDir, cacheDir)
#[no_mangle]
pub extern "system" fn Java_app_fotos_native_FotosNative_init(
    mut env: JNIEnv,
    _class: JClass,
    files_dir: JString,
    cache_dir: JString,
) {
    let _files_dir: String = env.get_string(&files_dir)
        .map(|s| s.into())
        .unwrap_or_default();
    let _cache_dir: String = env.get_string(&cache_dir)
        .map(|s| s.into())
        .unwrap_or_default();

    // Initialize the photo service
    let _ = get_photo_service();
}

/// Process a photo from MediaStore
/// Called from Kotlin: FotosNative.processPhoto(photoData, contentUri, dbPath, thumbDir)
#[no_mangle]
pub extern "system" fn Java_app_fotos_native_FotosNative_processPhoto(
    mut env: JNIEnv,
    _class: JClass,
    photo_data: JByteArray,
    content_uri: JString,
    db_path: JString,
    thumb_dir: JString,
) -> jint {
    let data = match env.convert_byte_array(photo_data) {
        Ok(d) => d,
        Err(_) => return -1,
    };

    let uri: String = match env.get_string(&content_uri) {
        Ok(s) => s.into(),
        Err(_) => return -2,
    };

    let db: String = match env.get_string(&db_path) {
        Ok(s) => s.into(),
        Err(_) => return -3,
    };

    let thumb: String = match env.get_string(&thumb_dir) {
        Ok(s) => s.into(),
        Err(_) => return -4,
    };

    match get_photo_service().process_photo(&data, &uri, &db, &thumb) {
        Ok(_) => 0,
        Err(_) => -5,
    }
}

/// Check if import should be cancelled
/// Called from Kotlin: FotosNative.isImportCancelled()
#[no_mangle]
pub extern "system" fn Java_app_fotos_native_FotosNative_isImportCancelled(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    get_photo_service().is_import_cancelled() as jboolean
}

/// Cancel the import operation
/// Called from Kotlin: FotosNative.cancelImport()
#[no_mangle]
pub extern "system" fn Java_app_fotos_native_FotosNative_cancelImport(
    _env: JNIEnv,
    _class: JClass,
) {
    get_photo_service().cancel_import();
}

/// Get photo count in database
/// Called from Kotlin: FotosNative.getPhotoCount(dbPath)
#[no_mangle]
pub extern "system" fn Java_app_fotos_native_FotosNative_getPhotoCount(
    mut env: JNIEnv,
    _class: JClass,
    db_path: JString,
) -> jlong {
    let db: String = match env.get_string(&db_path) {
        Ok(s) => s.into(),
        Err(_) => return -1,
    };

    use fotos_core::PhotoIndex;
    match PhotoIndex::open(db) {
        Ok(index) => match index.list() {
            Ok(photos) => photos.len() as jlong,
            Err(_) => -2,
        },
        Err(_) => -3,
    }
}

/// Clear all app data
/// Called from Kotlin: FotosNative.clearAppData(dbPath, thumbDir, rawPreviewDir, tileCacheDir)
#[no_mangle]
pub extern "system" fn Java_app_fotos_native_FotosNative_clearAppData(
    mut env: JNIEnv,
    _class: JClass,
    db_path: JString,
    thumb_dir: JString,
    raw_preview_dir: JString,
    tile_cache_dir: JString,
) -> jint {
    let db: String = env.get_string(&db_path).map(|s| s.into()).unwrap_or_default();
    let thumb: String = env.get_string(&thumb_dir).map(|s| s.into()).unwrap_or_default();
    let raw: String = env.get_string(&raw_preview_dir).map(|s| s.into()).unwrap_or_default();
    let tile: String = env.get_string(&tile_cache_dir).map(|s| s.into()).unwrap_or_default();

    use std::path::Path;
    use std::fs;

    // Delete database
    if Path::new(&db).exists() {
        if fs::remove_file(&db).is_err() {
            return -1;
        }
    }

    // Delete directories
    for dir in [&thumb, &raw, &tile] {
        if Path::new(dir).exists() {
            if fs::remove_dir_all(dir).is_err() {
                return -2;
            }
        }
    }

    0
}
