fn main() {
    // The macOS app draws the map with the system map view; that part is
    // Objective-C (macos/FootosMacMap.m).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rerun-if-changed=macos/FootosMacMap.m");
        cc::Build::new()
            .file("macos/FootosMacMap.m")
            .flag("-fobjc-arc")
            .flag("-fmodules")
            .compile("footosmacmap");
        for framework in ["MapKit", "CoreLocation", "AppKit", "WebKit", "QuartzCore", "ImageIO"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }
    tauri_build::build()
}
