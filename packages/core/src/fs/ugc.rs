//! Telling a person's own photos and videos from everything else on a disk.
//!
//! Pointing the app at a whole drive used to walk every system and
//! application folder and import icons, textures and web graphics. Two
//! filters keep the library to what a person shot:
//!
//! 1. Folders that hold software rather than documents are not entered:
//!    system and application folders, caches, and source-code projects.
//! 2. A picture has to be a photograph: taken by a camera (it says so in
//!    its metadata) or at least the size of one. Photos passed around in
//!    chat apps have no camera metadata left, so that alone cannot be the
//!    test. A video has to be more than a clip of UI.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::error::CoreError;
use crate::image::parse::is_raw_file;
use crate::types::PhotoMetadata;

use super::scan::{is_supported_image, is_supported_video};

/// Videos smaller than this are UI animations and stickers far more often
/// than something a person filmed.
pub const MIN_VIDEO_BYTES: u64 = 1024 * 1024;

/// A picture without camera metadata still counts when it is at least this
/// big: icons, thumbnails and UI art are smaller.
pub const MIN_PHOTO_LONG_SIDE: u32 = 480;
pub const MIN_PHOTO_SHORT_SIDE: u32 = 320;
pub const MIN_PHOTO_BYTES: u64 = 20 * 1024;

/// macOS shows the data volume twice: at `/` and under this prefix. Both
/// name the same files.
const DATA_VOLUME_ALIAS: &str = "/System/Volumes/Data";

/// The path with the macOS data-volume alias removed, so that one file has
/// one name.
pub fn without_volume_alias(path: &str) -> &str {
    match path.strip_prefix(DATA_VOLUME_ALIAS) {
        Some(rest) if rest.starts_with('/') => rest,
        _ => path,
    }
}

/// Files that mark a folder as a source-code project. Pictures inside one
/// are assets, fixtures and screenshots of the software.
const PROJECT_MARKERS: &[&str] = &[
    ".git", ".svn", ".hg", "package.json", "Cargo.toml", "go.mod", "pom.xml", "build.gradle",
    "build.gradle.kts", "pubspec.yaml", "Podfile", "pyproject.toml", "requirements.txt",
    "CMakeLists.txt", "Gemfile", "composer.json",
];
const PROJECT_MARKER_SUFFIXES: &[&str] = &[".xcodeproj", ".xcworkspace", ".sln", ".csproj"];

/// Whether `dir` is the top of a source-code project.
pub fn is_project_dir(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    entries.filter_map(Result::ok).any(|entry| {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { return false };
        PROJECT_MARKERS.contains(&name) || PROJECT_MARKER_SUFFIXES.iter().any(|s| name.ends_with(s))
    })
}

/// Folder names that never hold a person's photos, wherever they appear.
const SKIP_NAMES: &[&str] = &[
    "node_modules",
    "bower_components",
    "site-packages",
    "__pycache__",
    "DerivedData",
    "Pods",
    "Caches",
    "Cache",
    "CachedData",
    "Application Support",
    "Containers",
    "Group Containers",
    "$RECYCLE.BIN",
    "System Volume Information",
    "steamapps",
];

/// Folders that are really one file to their owner (bundles and packages).
const SKIP_SUFFIXES: &[&str] = &[
    ".app",
    ".framework",
    ".bundle",
    ".plugin",
    ".kext",
    ".xcassets",
    ".xcodeproj",
    ".lproj",
    ".imageset",
    ".appiconset",
    ".pkg",
    ".docset",
];

/// System folders, recognised only at the top of a volume.
const SKIP_AT_VOLUME_ROOT: &[&str] = &[
    "System", "Library", "Applications", "usr", "bin", "sbin", "private", "opt", "cores",
    "dev", "etc", "tmp", "var", "proc", "sys", "run", "snap", "boot", "lib", "lib64",
    "Windows", "Program Files", "Program Files (x86)", "ProgramData",
];

/// Per-user folders that belong to software.
const SKIP_IN_HOME: &[&str] = &["Library", "Applications", "AppData"];

fn is_volume_root(dir: &Path) -> bool {
    let mut parts = dir.components();
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        // "/" and "C:\"
        (Some(std::path::Component::RootDir), None, _, _) => true,
        (Some(std::path::Component::Prefix(_)), Some(std::path::Component::RootDir), None, _) => true,
        // "/Volumes/<name>" and "/mnt/<name>", "/media/<name>"
        (Some(std::path::Component::RootDir), Some(a), Some(_), None) => {
            matches!(a.as_os_str().to_str(), Some("Volumes" | "mnt" | "media"))
        }
        _ => false,
    }
}

fn is_home(dir: &Path) -> bool {
    let Some(parent) = dir.parent() else { return false };
    matches!(
        parent.file_name().and_then(|n| n.to_str()),
        Some("Users" | "home")
    ) && parent.parent().map(is_volume_root).unwrap_or(false)
}

/// Whether a scan should stay out of `dir`. `dir` is absolute.
pub fn is_software_dir(dir: &Path) -> bool {
    let Some(name) = dir.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name.starts_with('.') || SKIP_NAMES.contains(&name) {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    if SKIP_SUFFIXES.iter().any(|s| lower.ends_with(s)) {
        return true;
    }
    let Some(parent) = dir.parent() else { return false };
    if is_volume_root(parent) && SKIP_AT_VOLUME_ROOT.contains(&name) {
        return true;
    }
    if is_home(parent) && SKIP_IN_HOME.contains(&name) {
        return true;
    }
    // A Photos library keeps the pictures in `originals` (older: `Masters`);
    // the rest is previews, thumbnails and databases.
    if parent
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.to_ascii_lowercase().ends_with(".photoslibrary"))
        .unwrap_or(false)
    {
        return !matches!(name, "originals" | "Masters");
    }
    false
}

/// Hidden files. On volumes that cannot hold extended attributes (the exFAT
/// of a memory card) macOS writes a 4 KB "._NAME" companion next to every
/// file it touches, with the same extension: 79 "RAW files" on one card.
fn is_hidden_file(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with('.')).unwrap_or(false)
}

/// File types that are graphics rather than photographs. Cameras and phones
/// write JPEG, HEIC or RAW; PNG and WebP are screenshots, icons and web art.
fn is_graphics_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|s| s.to_str()).map(|s| s.to_ascii_lowercase()).as_deref(),
        Some("png" | "webp")
    )
}

/// The rules of a folder scan, with what was learned about folders kept so
/// that judging many paths does not look at the same folder twice.
#[derive(Default)]
pub struct ScanRules {
    skipped: HashMap<PathBuf, bool>,
}

impl ScanRules {
    /// Whether a scan stays out of `dir` (a software folder or a code project).
    pub fn skips_dir(&mut self, dir: &Path) -> bool {
        if let Some(&known) = self.skipped.get(dir) {
            return known;
        }
        let skip = is_software_dir(dir) || is_project_dir(dir);
        self.skipped.insert(dir.to_path_buf(), skip);
        skip
    }

    /// Whether a scan of `root` leaves `path` out by rule (not because it
    /// could not be read). `root` itself is always entered: the person chose it.
    pub fn excludes(&mut self, root: &Path, path: &Path) -> bool {
        if is_graphics_file(path) || is_hidden_file(path) {
            return true;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            return false;
        };
        let mut dir = root.to_path_buf();
        let mut parts: Vec<_> = relative.components().collect();
        parts.pop(); // the file name
        for part in parts {
            dir.push(part);
            if self.skips_dir(&dir) {
                return true;
            }
        }
        false
    }
}

/// Like `scan_photos`, limited to where a person keeps photos and to file
/// types a camera writes. Symbolic links are not followed.
pub fn scan_user_media(root: &Path) -> Result<Vec<PathBuf>, CoreError> {
    let mut result = Vec::new();
    let mut rules = ScanRules::default();
    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(move |entry| {
            entry.depth() == 0 || !entry.file_type().is_dir() || !rules.skips_dir(entry.path())
        });
    for entry in walker.filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if is_graphics_file(path) || is_hidden_file(path) || !(is_supported_image(path) || is_supported_video(path)) {
            continue;
        }
        if entry.metadata().map(|m| m.len() > 0).unwrap_or(false) {
            result.push(path.to_path_buf());
        }
    }
    Ok(result)
}

/// Whether the metadata is what a camera or phone writes.
pub fn has_camera_metadata(meta: &PhotoMetadata) -> bool {
    meta.make.is_some()
        || meta.model.is_some()
        || meta.date_taken.is_some()
        || meta.iso.is_some()
        || meta.f_number.is_some()
        || meta.exposure_time.is_some()
        || (meta.lat.is_some() && meta.lon.is_some())
}

fn is_heif(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|s| s.to_str()).map(|s| s.to_ascii_lowercase()).as_deref(),
        Some("heic" | "heif")
    )
}

/// Whether a picture of these dimensions and size is big enough to be a photograph.
pub fn is_photo_sized(width: u32, height: u32, file_size: u64) -> bool {
    file_size >= MIN_PHOTO_BYTES
        && width.max(height) >= MIN_PHOTO_LONG_SIDE
        && width.min(height) >= MIN_PHOTO_SHORT_SIDE
}

/// Whether a file found by a folder scan is something a person shot.
/// Reads the picture's header when the metadata does not settle it.
pub fn is_user_media(path: &Path, meta: &PhotoMetadata, file_size: u64) -> bool {
    if is_supported_video(path) {
        return file_size >= MIN_VIDEO_BYTES;
    }
    if is_graphics_file(path) || is_hidden_file(path) {
        return false;
    }
    // Only phones and cameras write these.
    if is_raw_file(path) || is_heif(path) || has_camera_metadata(meta) {
        return true;
    }
    match image::image_dimensions(path) {
        Ok((width, height)) => is_photo_sized(width, height, file_size),
        // Unreadable here does not mean it is not a photo.
        Err(_) => file_size >= MIN_PHOTO_BYTES * 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn software_folders_are_recognised() {
        for dir in [
            "/System",
            "/Library",
            "/Applications",
            "/usr",
            "/Users/me/Library",
            "/Users/me/Applications",
            "/Users/me/code/app/node_modules",
            "/Users/me/.cache",
            "/Users/me/Downloads/Thing.app",
            "/Users/me/proj/Assets.xcassets",
            "/Volumes/Disk/System",
            "/Users/me/Pictures/Photos Library.photoslibrary/resources",
        ] {
            assert!(is_software_dir(Path::new(dir)), "{dir} should be skipped");
        }
        for dir in [
            "/Users",
            "/Users/me",
            "/Users/me/Pictures",
            "/Users/me/Pictures/2024/Library",
            "/Volumes/Disk/Photos",
            "/Users/me/Pictures/Photos Library.photoslibrary",
            "/Users/me/Pictures/Photos Library.photoslibrary/originals",
            "/Users/me/Desktop/System",
        ] {
            assert!(!is_software_dir(Path::new(dir)), "{dir} should be entered");
        }
    }

    #[test]
    fn exclusion_is_judged_below_the_chosen_root_only() {
        let mut rules = ScanRules::default();
        // The person chose a folder inside ~/Library: it is scanned.
        let root = Path::new("/Users/me/Library/Mobile Documents");
        assert!(!rules.excludes(root, &root.join("trip/a.jpg")));
        assert!(rules.excludes(root, &root.join("x.app/icon.jpg")));
        assert!(rules.excludes(Path::new("/"), Path::new("/Users/me/Library/a.jpg")));
        assert!(rules.excludes(Path::new("/"), Path::new("/Users/me/Pictures/shot.png")));
        assert!(!rules.excludes(Path::new("/"), Path::new("/Users/me/Pictures/a.jpg")));
        assert!(rules.excludes(Path::new("/Volumes/Card"), Path::new("/Volumes/Card/DCIM/._DSC_1.NEF")));
    }

    #[test]
    fn the_data_volume_alias_is_the_same_place() {
        assert_eq!(without_volume_alias("/System/Volumes/Data/Users/me/a.jpg"), "/Users/me/a.jpg");
        assert_eq!(without_volume_alias("/Users/me/a.jpg"), "/Users/me/a.jpg");
        assert_eq!(without_volume_alias("/System/Volumes/DataX/a.jpg"), "/System/Volumes/DataX/a.jpg");
    }

    #[test]
    fn scan_stays_out_of_software_folders_and_skips_graphics() {
        let root = std::env::temp_dir().join("footos_ugc_scan_test");
        let _ = fs::remove_dir_all(&root);
        for dir in ["trip", "proj/node_modules/pkg", "Tool.app/Contents", ".hidden", "repo/assets"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in [
            "trip/a.jpg",
            "trip/b.HEIC",
            "trip/clip.mov",
            "trip/screenshot.png",
            "proj/node_modules/pkg/logo.jpg",
            "Tool.app/Contents/bg.jpg",
            ".hidden/c.jpg",
            "trip/._a.jpg",
            "repo/assets/hero.jpg",
            "repo/Cargo.toml",
        ] {
            fs::write(root.join(file), b"data").unwrap();
        }
        let mut found: Vec<String> = scan_user_media(&root)
            .unwrap()
            .iter()
            .map(|p| p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"))
            .collect();
        found.sort();
        assert_eq!(found, vec!["trip/a.jpg", "trip/b.HEIC", "trip/clip.mov"]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn camera_pictures_photo_sized_pictures_and_real_videos_count() {
        let bare = PhotoMetadata::default();
        let shot = PhotoMetadata { make: Some("Canon".into()), ..Default::default() };
        // These paths do not exist: only rules that need no file are checked.
        assert!(is_user_media(Path::new("a.jpg"), &shot, 500_000));
        assert!(is_user_media(Path::new("a.heic"), &bare, 500_000));
        assert!(is_user_media(Path::new("a.dng"), &bare, 500_000));
        assert!(!is_user_media(Path::new("a.png"), &shot, 500_000));
        assert!(!is_user_media(Path::new("a.mp4"), &bare, 200_000));
        assert!(is_user_media(Path::new("a.mp4"), &bare, 5_000_000));
        // A chat app's copy of a photo: no metadata, but the size of one.
        assert!(is_photo_sized(1080, 1440, 300_000));
        assert!(is_photo_sized(640, 480, 60_000));
        // Icons and thumbnails.
        assert!(!is_photo_sized(256, 256, 60_000));
        assert!(!is_photo_sized(1024, 200, 60_000));
        assert!(!is_photo_sized(800, 600, 4_000));
    }
}
