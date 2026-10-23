//! The `cloudflared` release this build of the gateway installs, and the SHA-256 of each of
//! its three Linux assets. Compile-time constants, so the Docker image carries them too.
//!
//! Rewritten by `tools/update-cloudflared-pin.sh <version>`, which downloads the three
//! assets from the release and sums them. Before each Lyrion Anywhere release, move this to
//! the newest `cloudflared` release (docs/RELEASING.md). The gateway refuses any download
//! whose sum differs, and `cloudflared` never updates itself (`--no-autoupdate`).

/// The pinned `cloudflared` release (its tag on GitHub, e.g. `2026.10.1`).
pub const CLOUDFLARED_VERSION: &str = "2026.10.3";

/// SHA-256 of `cloudflared-linux-amd64` (x86_64).
pub const SHA256_AMD64: &str = "053b80b0a23031a038fdbf3c073f59f82b8f1f152aa67971d7383ffafc0dbab0";

/// SHA-256 of `cloudflared-linux-arm64` (aarch64).
pub const SHA256_ARM64: &str = "89297ebfb9e8c99c0dc013a1140601453cf3798d8306c5591651f4a3f5210be8";

/// SHA-256 of `cloudflared-linux-armhf` (32-bit ARM, hard float).
pub const SHA256_ARMHF: &str = "458928ef2729ad71ebf7c88c289f9ecb2ac8bc6810a3bd4d70778592b9b4268b";

#[cfg(test)]
mod tests {
    use super::*;

    /// `^\d{4}\.\d{1,2}\.\d+$`
    fn is_version(v: &str) -> bool {
        let parts: Vec<&str> = v.split('.').collect();
        let digits = |s: &str, min: usize, max: usize| {
            (min..=max).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
        };
        parts.len() == 3
            && digits(parts[0], 4, 4)
            && digits(parts[1], 1, 2)
            && digits(parts[2], 1, usize::MAX)
    }

    #[test]
    fn the_pinned_version_is_a_cloudflared_release_number() {
        assert!(is_version(CLOUDFLARED_VERSION), "{CLOUDFLARED_VERSION}");
        assert!(is_version("2026.1.0"));
        assert!(!is_version("2026.10"));
        assert!(!is_version("v2026.10.1"));
        assert!(!is_version("2026.100.1"));
        assert!(!is_version("2026.10.1-rc1"));
    }

    #[test]
    fn each_pinned_sum_is_64_lower_case_hex_characters() {
        for sum in [SHA256_AMD64, SHA256_ARM64, SHA256_ARMHF] {
            assert_eq!(sum.len(), 64, "{sum}");
            assert!(
                sum.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "{sum}"
            );
        }
        assert_ne!(SHA256_AMD64, SHA256_ARM64);
        assert_ne!(SHA256_ARM64, SHA256_ARMHF);
        assert_ne!(SHA256_AMD64, SHA256_ARMHF);
    }
}
