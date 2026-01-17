# Proguard rules for 足迹相册 (footos) Android app

# Keep JNI native methods
-keepclasseswithmembernames class * {
    native <methods>;
}

# Keep FootosNative JNI class
-keep class app.footos.native.FootosNative { *; }

# Keep JavaScript interface methods
-keepclassmembers class app.footos.bridge.FootosWebViewBridge {
    @android.webkit.JavascriptInterface <methods>;
}

# Keep all bridge classes
-keep class app.footos.bridge.** { *; }

# Keep MainActivity
-keep class app.footos.MainActivity { *; }

# Suppress warnings for Kotlin coroutines
-dontwarn kotlinx.coroutines.**
