# UniFFI generates reflective JNA bindings. R8 must keep both.
-keep class uniffi.** { *; }
-keep class com.sun.jna.** { *; }
-dontwarn com.sun.jna.**
-dontwarn java.awt.**
