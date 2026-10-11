# UniFFI bindings are called through JNA, which finds classes and methods by reflection.
-keep class com.sun.jna.** { *; }
-keep class * extends com.sun.jna.** { *; }
-dontwarn java.awt.**
-keep class io.github.jamezuk.lyrionanywhere.core.** { *; }

# The Rust library calls Android's certificate verifier through JNI.
-keep, includedescriptorclasses class org.rustls.platformverifier.** { *; }

# Called from Kotlin into Rust by its JNI name.
-keep class io.github.jamezuk.lyrionanywhere.rust.NativeInit { *; }
