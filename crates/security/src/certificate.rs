// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;

use rustls::ServerConfig;
use rustls::crypto::ring;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

use crate::Fingerprint;
use crate::store::{create_private_directory, write_private};

const CERTIFICATE_FILE: &str = "certificate.der";
const KEY_FILE: &str = "key.der";

/// The server's self-signed certificate and its private key, which persist across restarts so that clients can pin the certificate. See `design/security.md`.
pub struct ServerCertificate {
    certificate: Vec<u8>,
    key: Vec<u8>,
}

impl ServerCertificate {
    /// Load the certificate from `directory`, or generate one and store it there, with its key readable only by the server's user.
    pub fn load_or_create(directory: &Path) -> io::Result<Self> {
        let certificate_path = directory.join(CERTIFICATE_FILE);
        let key_path = directory.join(KEY_FILE);
        match (fs::read(&certificate_path), fs::read(&key_path)) {
            (Ok(certificate), Ok(key)) => return Ok(Self { certificate, key }),
            (Err(error), _) | (_, Err(error)) if error.kind() != io::ErrorKind::NotFound => {
                return Err(error);
            }
            _ => {}
        }
        create_private_directory(directory)?;
        let generated = Self::generate();
        write_private(&key_path, &generated.key)?;
        fs::write(&certificate_path, &generated.certificate)?;
        Ok(generated)
    }

    /// A new certificate and key. Clients pin the certificate rather than check its names, so its name is only descriptive.
    pub fn generate() -> Self {
        let generated = rcgen::generate_simple_self_signed(vec!["tomori".to_owned()])
            .expect("a self-signed certificate for a fixed name can be generated");
        Self {
            certificate: generated.cert.der().to_vec(),
            key: generated.key_pair.serialize_der(),
        }
    }

    /// The fingerprint clients pin, as advertised in the server's Bonjour TXT record.
    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint::of(&self.certificate)
    }

    /// A TLS server configuration presenting this certificate.
    pub fn server_config(&self) -> Result<Arc<ServerConfig>, rustls::Error> {
        let config = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("ring supports the default protocol versions")
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(self.certificate.clone())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key.clone())),
            )?;
        Ok(Arc::new(config))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rustls::pki_types::ServerName;
    use rustls::{ClientConnection, ServerConnection};

    use super::*;
    use crate::{Pinned, client_config};

    fn directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "tomori-certificate-{name}-{}",
            crate::hex::random(4)
        ));
        let _ = fs::remove_dir_all(&directory);
        directory
    }

    /// Run a handshake in memory between `server` and a client pinning `expected`.
    fn handshake(
        server: &ServerCertificate,
        expected: Option<Fingerprint>,
    ) -> (Result<(), rustls::Error>, Arc<Pinned>) {
        handshake_with(server.server_config().unwrap(), expected)
    }

    fn handshake_with(
        config: Arc<ServerConfig>,
        expected: Option<Fingerprint>,
    ) -> (Result<(), rustls::Error>, Arc<Pinned>) {
        let pinned = Pinned::new(expected);
        let mut client = ClientConnection::new(
            client_config(pinned.clone()),
            ServerName::try_from("tomori").unwrap(),
        )
        .unwrap();
        let mut server = ServerConnection::new(config).unwrap();
        let result = (|| {
            while client.is_handshaking() || server.is_handshaking() {
                let mut buffer = Vec::new();
                client.write_tls(&mut buffer).unwrap();
                server.read_tls(&mut buffer.as_slice()).unwrap();
                server.process_new_packets().expect(
                    "the server accepts what the client sends before the client refuses it",
                );
                let mut buffer = Vec::new();
                server.write_tls(&mut buffer).unwrap();
                client.read_tls(&mut buffer.as_slice()).unwrap();
                client.process_new_packets()?;
            }
            Ok(())
        })();
        (result, pinned)
    }

    #[test]
    fn the_certificate_persists_and_pins() {
        let directory = directory("persist");
        let certificate = ServerCertificate::load_or_create(&directory).unwrap();
        let again = ServerCertificate::load_or_create(&directory).unwrap();
        assert_eq!(certificate.fingerprint(), again.fingerprint());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(directory.join(KEY_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        // A client pinning the fingerprint completes the handshake, and so does one with no pin yet, which records it.
        let (result, pinned) = handshake(&certificate, Some(certificate.fingerprint()));
        result.unwrap();
        assert_eq!(pinned.seen(), Some(certificate.fingerprint()));
        let (result, pinned) = handshake(&certificate, None);
        result.unwrap();
        assert_eq!(pinned.seen(), Some(certificate.fingerprint()));

        // A different certificate is refused.
        let other = ServerCertificate::generate();
        assert_ne!(other.fingerprint(), certificate.fingerprint());
        let (result, _) = handshake(&other, Some(certificate.fingerprint()));
        assert!(result.is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_tls_1_2_server_is_verified_too() {
        let certificate = ServerCertificate::generate();
        let config = ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS12])
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(certificate.certificate.clone())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(certificate.key.clone())),
            )
            .unwrap();
        let (result, _) = handshake_with(Arc::new(config), Some(certificate.fingerprint()));
        result.unwrap();
    }

    #[test]
    fn an_unreadable_certificate_or_key_is_an_error() {
        let directory = directory("unreadable");
        fs::create_dir_all(directory.join(CERTIFICATE_FILE)).unwrap();
        assert!(ServerCertificate::load_or_create(&directory).is_err());
        fs::remove_dir_all(&directory).unwrap();

        // A missing certificate is generated again, but a key that cannot be read is not replaced.
        fs::create_dir_all(directory.join(KEY_FILE)).unwrap();
        assert!(ServerCertificate::load_or_create(&directory).is_err());
        assert!(directory.join(KEY_FILE).is_dir());
        assert!(!directory.join(CERTIFICATE_FILE).exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_certificate_that_cannot_be_stored_is_an_error() {
        let parent = directory("unstored");
        fs::create_dir_all(&parent).unwrap();

        // The key cannot be written where a directory has its temporary file's name, and then no certificate is written either.
        let directory = parent.join("state");
        fs::create_dir_all(directory.join(KEY_FILE).with_extension("tmp")).unwrap();
        assert!(ServerCertificate::load_or_create(&directory).is_err());
        assert!(!directory.join(KEY_FILE).exists());
        assert!(!directory.join(CERTIFICATE_FILE).exists());
        fs::remove_dir(directory.join(KEY_FILE).with_extension("tmp")).unwrap();

        #[cfg(unix)]
        {
            // The directory cannot be created where a link to a missing one has its name, though nothing can be read there either.
            let linked = parent.join("linked");
            std::os::unix::fs::symlink(parent.join("missing"), &linked).unwrap();
            assert!(ServerCertificate::load_or_create(&linked).is_err());
            assert!(!parent.join("missing").exists());

            // The certificate cannot be written through a link to a directory that does not exist.
            std::os::unix::fs::symlink(
                parent.join("missing").join(CERTIFICATE_FILE),
                directory.join(CERTIFICATE_FILE),
            )
            .unwrap();
            assert!(ServerCertificate::load_or_create(&directory).is_err());
            fs::remove_file(directory.join(CERTIFICATE_FILE)).unwrap();
        }

        // Once it can be stored, it is.
        let certificate = ServerCertificate::load_or_create(&directory).unwrap();
        assert_eq!(
            ServerCertificate::load_or_create(&directory)
                .unwrap()
                .fingerprint(),
            certificate.fingerprint()
        );
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn a_corrupted_key_cannot_be_served() {
        let directory = directory("corrupted");
        let certificate = ServerCertificate::load_or_create(&directory).unwrap();
        fs::write(directory.join(KEY_FILE), b"not a key").unwrap();
        let loaded = ServerCertificate::load_or_create(&directory).unwrap();
        assert_eq!(loaded.fingerprint(), certificate.fingerprint());
        assert!(loaded.server_config().is_err());
        fs::remove_dir_all(directory).unwrap();
    }
}
