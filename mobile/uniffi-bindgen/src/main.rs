//! Generates the Kotlin (and later Swift) bindings for `anywhere-mobile`. Run by the
//! Android build (the `:rust` module); pinned to the same UniFFI version as the library.
fn main() {
    uniffi::uniffi_bindgen_main()
}
