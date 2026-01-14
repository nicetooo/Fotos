# Proguard rules for Fotos Android app

# Keep JNI native methods
-keepclasseswithmembernames class * {
    native <methods>;
}

# Keep FotosNative JNI class
-keep class app.fotos.native.FotosNative { *; }

# Keep JavaScript interface methods
-keepclassmembers class app.fotos.bridge.FotosWebViewBridge {
    @android.webkit.JavascriptInterface <methods>;
}

# Keep all bridge classes
-keep class app.fotos.bridge.** { *; }

# Keep MainActivity
-keep class app.fotos.MainActivity { *; }

# Suppress warnings for Kotlin coroutines
-dontwarn kotlinx.coroutines.**
