// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, ring};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, Error, SignatureScheme};

use crate::Fingerprint;

/// Accepts a server's certificate by its fingerprint rather than by a certificate authority, since a Tomori server's certificate is self-signed. With no fingerprint expected, the first certificate seen is accepted, and a client then remembers it. See `design/security.md`.
///
/// The handshake signature is still verified against the certificate's key, so a server that presents a certificate must hold its private key.
#[derive(Debug)]
pub struct Pinned {
    expected: Option<Fingerprint>,
    seen: Mutex<Option<Fingerprint>>,
    provider: Arc<CryptoProvider>,
}

impl Pinned {
    /// Accept only the certificate with `expected`'s fingerprint, or, without one, whichever certificate the server presents.
    pub fn new(expected: Option<Fingerprint>) -> Arc<Self> {
        Arc::new(Self {
            expected,
            seen: Mutex::new(None),
            provider: Arc::new(ring::default_provider()),
        })
    }

    /// The fingerprint accepted, if one is pinned.
    pub fn expected(&self) -> Option<Fingerprint> {
        self.expected
    }

    /// The fingerprint of the certificate the server last presented, whether or not it was accepted.
    pub fn seen(&self) -> Option<Fingerprint> {
        *self.seen.lock().expect("the pin is never left poisoned")
    }
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let fingerprint = Fingerprint::of(end_entity);
        *self.seen.lock().expect("the pin is never left poisoned") = Some(fingerprint);
        match self.expected {
            Some(expected) if expected != fingerprint => Err(Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            )),
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            certificate,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            certificate,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// A TLS client configuration that trusts only what `pinned` accepts.
pub fn client_config(pinned: Arc<Pinned>) -> Arc<ClientConfig> {
    Arc::new(
        ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("the ring provider supports the default protocol versions")
            .dangerous()
            .with_custom_certificate_verifier(pinned)
            .with_no_client_auth(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verify(pinned: &Pinned, certificate: &[u8]) -> Result<ServerCertVerified, Error> {
        pinned.verify_server_cert(
            &CertificateDer::from(certificate.to_vec()),
            &[],
            &ServerName::try_from("tomori").unwrap(),
            &[],
            UnixTime::now(),
        )
    }

    #[test]
    fn accepts_only_the_pinned_certificate() {
        let pinned = Pinned::new(Some(Fingerprint::of(b"ours")));
        assert_eq!(pinned.expected(), Some(Fingerprint::of(b"ours")));
        assert_eq!(pinned.seen(), None);
        assert!(verify(&pinned, b"ours").is_ok());
        assert_eq!(pinned.seen(), Some(Fingerprint::of(b"ours")));
        assert_eq!(
            verify(&pinned, b"theirs").unwrap_err(),
            Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure)
        );
        assert_eq!(pinned.seen(), Some(Fingerprint::of(b"theirs")));
    }

    #[test]
    fn without_a_pin_accepts_and_records_the_first_certificate() {
        let pinned = Pinned::new(None);
        assert!(verify(&pinned, b"first").is_ok());
        assert_eq!(pinned.seen(), Some(Fingerprint::of(b"first")));
        assert!(!pinned.supported_verify_schemes().is_empty());
        client_config(pinned);
    }
}
