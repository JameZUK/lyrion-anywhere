//! The `cloudflared` release this build of the gateway installs, and the SHA-256 of each of
//! its three Linux assets. Compile-time constants, so the Docker image carries them too.
//!
//! Rewritten by `tools/update-cloudflared-pin.sh <version>`, which downloads the three
//! assets from the release and sums them. Before each Lyrion Anywhere release, move this to
//! the newest `cloudflared` release (docs/RELEASING.md). The gateway refuses any download
//! whose sum differs, and `cloudflared` never updates itself (`--no-autoupdate`).

/// The pinned `cloudflared` release (its tag on GitHub, e.g. `2026.10.1`).
pub const CLOUDFLARED_VERSION: &str = "2026.10.1";

/// SHA-256 of `cloudflared-linux-amd64` (x86_64).
pub const SHA256_AMD64: &str = "3bfc539f45d2620c71debb7a86be0efa737ed5ccbad9eb85c787116900e5c586";

/// SHA-256 of `cloudflared-linux-arm64` (aarch64).
pub const SHA256_ARM64: &str = "2c65dfabf616c40efc7545d107d7a57ad5e21377e2964b7e88af19a238074a46";

/// SHA-256 of `cloudflared-linux-armhf` (32-bit ARM, hard float).
pub const SHA256_ARMHF: &str = "c047e24af9ed956b55d514e3a0e2d8020e9c6d0629c4731c8ec08bd79a29a050";

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
