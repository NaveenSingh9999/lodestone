# Keep JNA machinery: the FFI interface is bound by reflection at runtime.
-keep class com.sun.jna.** { *; }
-keep class com.sun.jna.platform.** { *; }
-dontwarn com.sun.jna.**
-keep interface com.sun.jna.Library { *; }
-keep interface com.sun.jna.Callback { *; }
-keep interface com.lodestone.app.native.LodestoneLib { *; }
-keep interface com.lodestone.app.native.EventCallback { *; }
