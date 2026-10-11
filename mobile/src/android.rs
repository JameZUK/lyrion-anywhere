//! Android start-up: hands the platform certificate verifier the JVM and an application
//! `Context`, so TLS certificates are checked by Android's own TrustManager (system and
//! user CAs as the platform's network security configuration allows).
//!
//! Called once from Kotlin (`io.github.jamezuk.lyrionanywhere.rust.NativeInit`) before any
//! network call.

use jni::EnvUnowned;
use jni::errors::ThrowRuntimeExAndDefault;
use jni::jni_mangle;
use jni::objects::{JClass, JObject};

#[jni_mangle("io.github.jamezuk.lyrionanywhere.rust.NativeInit")]
pub fn init_platform_verifier<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    context: JObject<'caller>,
) {
    env.with_env(|env| rustls_platform_verifier::android::init_with_env(env, context))
        .resolve::<ThrowRuntimeExAndDefault>();
}
