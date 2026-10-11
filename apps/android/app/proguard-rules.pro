# The Rust bindings' keep rules come from the :rust module (consumer-rules.pro).

# Type-safe navigation routes are serialised by kotlinx.serialization.
-keepclassmembers @kotlinx.serialization.Serializable class io.github.jamezuk.lyrionanywhere.** {
    *** Companion;
    kotlinx.serialization.KSerializer serializer(...);
}

# Glance creates widget action callbacks by class name.
-keep class * implements androidx.glance.appwidget.action.ActionCallback { <init>(); }
