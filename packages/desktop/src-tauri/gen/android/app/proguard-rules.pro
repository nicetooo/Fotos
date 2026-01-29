# Proguard rules for 足迹相册 (footos) Android app

# === JNI / Native ===
-keepclasseswithmembernames class * {
    native <methods>;
}
-keep class app.footos.native.FootosNative { *; }

# === Tauri generated classes (called via JNI/reflection) ===
-keep class app.footos.generated.** { *; }
-keep class app.footos.MainActivity { *; }

# === App bridge classes ===
-keep class app.footos.bridge.** { *; }
-keepclassmembers class app.footos.bridge.FootosWebViewBridge {
    @android.webkit.JavascriptInterface <methods>;
}

# === WebView / JavaScript interface ===
-keepclassmembers class * {
    @android.webkit.JavascriptInterface <methods>;
}
-keepattributes JavascriptInterface

# === AndroidX WebKit ===
-keep class androidx.webkit.** { *; }

# === Tauri plugins (dialog, opener) ===
-keep class app.tauri.** { *; }
-keep class app.tauri.plugin.** { *; }

# === Kotlin / Coroutines ===
-dontwarn kotlinx.coroutines.**
-keepnames class kotlinx.coroutines.internal.MainDispatcherFactory {}
-keepnames class kotlinx.coroutines.CoroutineExceptionHandler {}
-keepclassmembers class kotlinx.coroutines.** {
    volatile <fields>;
}

# === Serialization (keep enum values) ===
-keepclassmembers enum * {
    public static **[] values();
    public static ** valueOf(java.lang.String);
}

# === Keep annotations ===
-keepattributes *Annotation*
-keepattributes Signature
-keepattributes InnerClasses,EnclosingMethod
