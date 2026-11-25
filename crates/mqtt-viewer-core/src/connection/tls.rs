//! TLS client configuration for the encrypted transports.

use std::path::Path;
use std::sync::Arc;

use rumqttc::tokio_rustls::rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rumqttc::tokio_rustls::rustls::crypto::{
    CryptoProvider, aws_lc_rs, verify_tls12_signature, verify_tls13_signature,
};
use rumqttc::tokio_rustls::rustls::pki_types::pem::PemObject as _;
use rumqttc::tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rumqttc::tokio_rustls::rustls::{ClientConfig, DigitallySignedStruct, Error, RootCertStore, SignatureScheme};

use crate::config::ConnectionProfile;

/// Builds the TLS configuration a profile asks for.
///
/// With certificate validation on, the system trust store plus the profile's CA file are trusted. With it off,
/// any server certificate is accepted (handshake signatures are still checked).
pub(super) fn client_config(profile: &ConnectionProfile) -> Result<ClientConfig, String> {
    let provider = Arc::new(aws_lc_rs::default_provider());
    let builder = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .map_err(|error| format!("TLS setup failed: {error}"))?;

    let builder = if profile.validate_certificate {
        let mut roots = RootCertStore::empty();
        for certificate in rustls_native_certs::load_native_certs().certs {
            // Unparsable system certificates are common and harmless; skip them.
            let _ = roots.add(certificate);
        }
        if let Some(ca_file) = &profile.ca_file {
            for certificate in read_certificates(ca_file)? {
                roots.add(certificate).map_err(|error| format!("CA file {}: {error}", ca_file.display()))?;
            }
        }
        builder.with_root_certificates(roots)
    } else {
        builder.dangerous().with_custom_certificate_verifier(Arc::new(AcceptAnyCertificate(provider)))
    };

    match (&profile.client_cert_file, &profile.client_key_file) {
        (Some(cert_file), Some(key_file)) => {
            let chain = read_certificates(cert_file)?;
            let key = PrivateKeyDer::from_pem_file(key_file)
                .map_err(|error| format!("client key {}: {error}", key_file.display()))?;
            builder.with_client_auth_cert(chain, key).map_err(|error| format!("client certificate: {error}"))
        }
        (None, None) => Ok(builder.with_no_client_auth()),
        (Some(_), None) | (None, Some(_)) => Err("client certificate and client key must be set together".to_owned()),
    }
}

fn read_certificates(path: &Path) -> Result<Vec<CertificateDer<'static>>, String> {
    let certificates = CertificateDer::pem_file_iter(path)
        .and_then(Iterator::collect::<Result<Vec<_>, _>>)
        .map_err(|error| format!("certificate file {}: {error}", path.display()))?;
    if certificates.is_empty() {
        return Err(format!("certificate file {} contains no certificates", path.display()));
    }
    Ok(certificates)
}

/// Verifier for "validate certificate: off": trusts every server certificate.
#[derive(Debug)]
struct AcceptAnyCertificate(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
