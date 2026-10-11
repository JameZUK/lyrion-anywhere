//! TLS certificate verification for the phone apps.
//!
//! Certificates are checked by the platform (rustls-platform-verifier: Android's
//! TrustManager, Apple's Security framework), so the phone's own trust store and policy
//! apply. One exception, on Android only:
//!
//! **"Revoked" from the Android verifier is not trusted on its own.** Its revocation step
//! reports *any* failure to check revocation as `Revoked`. Android can't fetch CRLs, so a
//! certificate that has a CRL distribution point but no OCSP responder (every Let's Encrypt
//! certificate since Let's Encrypt ended OCSP in 2025) is reported as revoked. When that
//! happens the certificate is verified again with webpki against the device's own system
//! CA store (chain, name, validity and usage, everything except online revocation). It is
//! accepted only if that passes. Every other platform verdict (unknown issuer, wrong name,
//! expired…) stands. This is the same level of revocation checking as Chrome on Android,
//! which doesn't check revocation online either.

// The fallback is built only on Android; elsewhere it is exercised by the tests alone.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, Error, RootCertStore, SignatureScheme};

/// Where Android keeps its system CA certificates: the updatable Conscrypt module (Android
/// 14+) first, then the system image.
#[cfg(target_os = "android")]
const ANDROID_CA_DIRS: &[&str] = &[
    "/apex/com.android.conscrypt/cacerts",
    "/system/etc/security/cacerts",
];

/// Most files read from a CA directory, and the largest one (a defence against an odd
/// directory, not a limit real stores come near).
const MAX_CA_FILES: usize = 1000;
const MAX_CA_FILE_BYTES: u64 = 64 * 1024;

/// The platform verifier, with a second opinion for its unreliable "revoked" verdict.
pub(crate) struct RevokedFallback {
    platform: Arc<dyn ServerCertVerifier>,
    fallback: Option<Arc<dyn ServerCertVerifier>>,
}

impl fmt::Debug for RevokedFallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RevokedFallback")
            .field("fallback", &self.fallback.is_some())
            .finish()
    }
}

impl RevokedFallback {
    pub(crate) fn new(
        platform: Arc<dyn ServerCertVerifier>,
        fallback: Option<Arc<dyn ServerCertVerifier>>,
    ) -> Self {
        Self { platform, fallback }
    }
}

impl ServerCertVerifier for RevokedFallback {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        match self.platform.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Err(Error::InvalidCertificate(CertificateError::Revoked)) => match &self.fallback {
                Some(second) => {
                    let ok = second.verify_server_cert(
                        end_entity,
                        intermediates,
                        server_name,
                        ocsp_response,
                        now,
                    )?;
                    log::info!(
                        "platform verifier reported revoked; the certificate verified against the system CA store"
                    );
                    Ok(ok)
                }
                None => Err(Error::InvalidCertificate(CertificateError::Revoked)),
            },
            other => other,
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.platform.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.platform.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.platform.supported_verify_schemes()
    }
}

/// Every CA certificate in `dir` (PEM files, as Android stores them, or raw DER), bounded.
pub(crate) fn load_ca_dir(dir: &Path) -> Vec<CertificateDer<'static>> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten().take(MAX_CA_FILES) {
        let path = entry.path();
        let small = entry
            .metadata()
            .map(|m| m.is_file() && m.len() <= MAX_CA_FILE_BYTES)
            .unwrap_or(false);
        if !small {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let pem: Vec<_> = CertificateDer::pem_slice_iter(&bytes)
            .filter_map(Result::ok)
            .collect();
        if pem.is_empty() {
            out.push(CertificateDer::from(bytes));
        } else {
            out.extend(pem);
        }
    }
    out
}

/// A webpki verifier trusting exactly `certs`, or `None` if none of them is usable.
pub(crate) fn webpki_verifier(
    certs: Vec<CertificateDer<'static>>,
    provider: Arc<CryptoProvider>,
) -> Option<Arc<dyn ServerCertVerifier>> {
    let mut roots = RootCertStore::empty();
    let (added, _ignored) = roots.add_parsable_certificates(certs);
    if added == 0 {
        return None;
    }
    WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider)
        .build()
        .ok()
        .map(|v| v as Arc<dyn ServerCertVerifier>)
}

/// The verifier the apps use: the platform's, with the Android fallback above.
pub(crate) fn verifier(
    provider: Arc<CryptoProvider>,
) -> Result<Arc<dyn ServerCertVerifier>, Error> {
    let platform: Arc<dyn ServerCertVerifier> =
        Arc::new(rustls_platform_verifier::Verifier::new(provider.clone())?);
    #[cfg(target_os = "android")]
    {
        let fallback = ANDROID_CA_DIRS
            .iter()
            .map(|d| load_ca_dir(Path::new(d)))
            .find(|certs| !certs.is_empty())
            .and_then(|certs| webpki_verifier(certs, provider));
        Ok(Arc::new(RevokedFallback::new(platform, fallback)))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = provider;
        Ok(platform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A verifier with a fixed answer, counting its calls.
    #[derive(Debug)]
    struct Fixed(Result<(), Error>, AtomicUsize);

    impl Fixed {
        fn new(r: Result<(), Error>) -> Arc<Self> {
            Arc::new(Self(r, AtomicUsize::new(0)))
        }
        fn calls(&self) -> usize {
            self.1.load(Ordering::SeqCst)
        }
    }

    impl ServerCertVerifier for Fixed {
        fn verify_server_cert(
            &self,
            _: &CertificateDer<'_>,
            _: &[CertificateDer<'_>],
            _: &ServerName<'_>,
            _: &[u8],
            _: UnixTime,
        ) -> Result<ServerCertVerified, Error> {
            self.1.fetch_add(1, Ordering::SeqCst);
            self.0.clone().map(|()| ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn verify_tls13_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![SignatureScheme::ED25519]
        }
    }

    fn check(v: &RevokedFallback) -> Result<ServerCertVerified, Error> {
        v.verify_server_cert(
            &CertificateDer::from(vec![1, 2, 3]),
            &[],
            &ServerName::try_from("music.example.com").unwrap(),
            &[],
            UnixTime::now(),
        )
    }

    const REVOKED: Error = Error::InvalidCertificate(CertificateError::Revoked);

    #[test]
    fn a_revoked_verdict_is_accepted_only_if_the_system_store_verifies_it() {
        // Regression: a valid Let's Encrypt certificate without OCSP was refused as revoked.
        let fb = Fixed::new(Ok(()));
        let v = RevokedFallback::new(Fixed::new(Err(REVOKED)), Some(fb.clone()));
        assert!(check(&v).is_ok());
        assert_eq!(fb.calls(), 1);

        let bad = Fixed::new(Err(Error::InvalidCertificate(
            CertificateError::UnknownIssuer,
        )));
        let v = RevokedFallback::new(Fixed::new(Err(REVOKED)), Some(bad));
        assert_eq!(
            check(&v).unwrap_err(),
            Error::InvalidCertificate(CertificateError::UnknownIssuer),
            "the second opinion's refusal stands"
        );
    }

    #[test]
    fn every_other_platform_verdict_stands() {
        for e in [
            Error::InvalidCertificate(CertificateError::UnknownIssuer),
            Error::InvalidCertificate(CertificateError::Expired),
            Error::InvalidCertificate(CertificateError::NotValidForName),
            Error::InvalidCertificate(CertificateError::BadSignature),
            Error::General("x".into()),
        ] {
            let fb = Fixed::new(Ok(()));
            let v = RevokedFallback::new(Fixed::new(Err(e.clone())), Some(fb.clone()));
            assert_eq!(check(&v).unwrap_err(), e);
            assert_eq!(fb.calls(), 0, "no second opinion for {e:?}");
        }
        let fb = Fixed::new(Err(REVOKED));
        let v = RevokedFallback::new(Fixed::new(Ok(())), Some(fb.clone()));
        assert!(check(&v).is_ok());
        assert_eq!(fb.calls(), 0);
    }

    #[test]
    fn without_a_system_store_revoked_stays_refused() {
        let v = RevokedFallback::new(Fixed::new(Err(REVOKED)), None);
        assert_eq!(check(&v).unwrap_err(), REVOKED);
    }

    #[test]
    fn signatures_and_schemes_come_from_the_platform() {
        let v = RevokedFallback::new(Fixed::new(Ok(())), None);
        assert_eq!(v.supported_verify_schemes(), vec![SignatureScheme::ED25519]);
    }

    #[test]
    fn ca_directories_are_read_pem_or_der_and_bounded() {
        let dir = std::env::temp_dir().join(format!("anywhere-ca-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Android's format: text, then a PEM block.
        std::fs::write(
            dir.join("a.0"),
            b"Certificate:\n  Data: ...\n-----BEGIN CERTIFICATE-----\nAQID\n-----END CERTIFICATE-----\n",
        )
        .unwrap();
        std::fs::write(dir.join("b.0"), [0x30, 0x03, 0x02, 0x01, 0x00]).unwrap();
        std::fs::write(
            dir.join("huge.0"),
            vec![0u8; (MAX_CA_FILE_BYTES + 1) as usize],
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("subdir")).unwrap();
        let mut certs = load_ca_dir(&dir);
        certs.sort_by(|a, b| a.as_ref().cmp(b.as_ref()));
        assert_eq!(
            certs.len(),
            2,
            "one PEM, one DER; the oversize file and the directory skipped"
        );
        assert_eq!(certs[0].as_ref(), &[1, 2, 3]);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(
            load_ca_dir(&dir).is_empty(),
            "a missing directory is empty, not an error"
        );
    }

    #[test]
    fn unusable_roots_give_no_fallback() {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        assert!(webpki_verifier(vec![], provider.clone()).is_none());
        assert!(webpki_verifier(vec![CertificateDer::from(vec![1, 2, 3])], provider).is_none());
    }

    #[test]
    fn a_real_root_store_verifies_a_real_chain_shape() {
        // The bundled Mozilla roots stand in for a device store: the builder accepts them.
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let roots = webpki_root_certs::TLS_SERVER_ROOT_CERTS.to_vec();
        assert!(webpki_verifier(roots, provider).is_some());
    }
}
