//! TLS options of the SISPRO client (`minsalud.ca_file`, `minsalud.tls_pin_sha256`).
//!
//! The local SISPRO server presents a certificate for `CN=localhost` without a
//! subject alternative name for its real host, so the usual chain and host name
//! checks cannot pass. Java trusted every certificate; here the operator either
//! gives the private CA (`ca_file`, handled by [`crate::http::build_client`]) or
//! the SHA-256 of the exact server certificate (`tls_pin_sha256`): the leaf is
//! compared with the pin and the handshake signature is still verified against
//! its key, so only the holder of that certificate can complete a handshake.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error, SignatureScheme};
use sha2::{Digest, Sha256};

use crate::http::client::DEFAULT_CONNECT_TIMEOUT;
use crate::http::{ClientOptions, build_client};

/// A SHA-256 certificate fingerprint.
pub type Pin = [u8; 32];

/// Parses a fingerprint given as 64 hex characters; colons, spaces and case are
/// ignored (so the `openssl x509 -fingerprint -sha256` output works as is).
pub fn parse_pin(text: &str) -> Result<Pin> {
    let hex: String = text
        .chars()
        .filter(|c| *c != ':' && !c.is_whitespace())
        .collect();
    let bytes = hex::decode(&hex).ok().filter(|b| b.len() == 32);
    match bytes {
        Some(bytes) => {
            let mut pin = [0u8; 32];
            pin.copy_from_slice(&bytes);
            Ok(pin)
        }
        None => bail!("minsalud.tls_pin_sha256 debe tener 64 caracteres hexadecimales (SHA-256)"),
    }
}

/// Trusts exactly one server certificate.
#[derive(Debug)]
struct PinVerifier {
    pin: Pin,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let seen: Pin = Sha256::digest(end_entity.as_ref()).into();
        if seen == self.pin {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(Error::General(
                "el certificado del servidor no coincide con minsalud.tls_pin_sha256".into(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// A rustls client configuration that accepts only the certificate with `pin`.
pub fn pinned_config(pin: Pin) -> Result<rustls::ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = PinVerifier {
        pin,
        algorithms: provider.signature_verification_algorithms,
    };
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .context("no se pudo configurar TLS")?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    Ok(config)
}

/// Builds the SISPRO HTTP client: system roots by default, `ca_file` to trust a
/// private CA, or `pin` to trust one exact certificate (not both).
pub fn build_sispro_client(
    ca_file: Option<&str>,
    pin: Option<&str>,
    timeout: Duration,
) -> Result<Client> {
    let ca_file = ca_file.filter(|p| !p.trim().is_empty());
    let pin = pin.filter(|p| !p.trim().is_empty());
    let Some(pin) = pin else {
        let options = ClientOptions::default()
            .with_ca_file(ca_file)
            .with_timeout(timeout);
        return build_client(&options);
    };
    if ca_file.is_some() {
        bail!("minsalud.ca_file y minsalud.tls_pin_sha256 son excluyentes: use solo uno");
    }
    let config = pinned_config(parse_pin(pin)?)?;
    Client::builder()
        .use_preconfigured_tls(config)
        .timeout(timeout)
        .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
        .build()
        .context("no se pudo crear el cliente HTTP")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_accept_openssl_fingerprints_and_plain_hex() {
        let colon = "8C:BB:7E:49:FA:BA:7C:61:48:8E:E8:0F:85:DE:99:0D:2D:0C:E8:2D:58:F2:82:FE:7D:B3:82:5D:75:07:AA:12";
        let plain = "8cbb7e49faba7c61488ee80f85de990d2d0ce82d58f282fe7db3825d7507aa12";
        assert_eq!(
            parse_pin(colon).expect("colon"),
            parse_pin(plain).expect("plain")
        );
        assert_eq!(
            parse_pin(&format!("  {plain}\n")).expect("spaces"),
            parse_pin(plain).expect("plain")
        );
    }

    #[test]
    fn malformed_pins_are_rejected_without_echoing_them() {
        for bad in ["", "abcd", "zz".repeat(32).as_str(), &"ab".repeat(33)] {
            let err = parse_pin(bad).expect_err("bad pin").to_string();
            assert!(err.contains("minsalud.tls_pin_sha256"), "{err}");
            assert!(bad.is_empty() || !err.contains(bad));
        }
    }

    #[test]
    fn a_ca_file_and_a_pin_exclude_each_other() {
        let plain = "8cbb7e49faba7c61488ee80f85de990d2d0ce82d58f282fe7db3825d7507aa12";
        let err = build_sispro_client(Some("/some/ca.pem"), Some(plain), Duration::from_secs(1))
            .expect_err("both");
        assert!(err.to_string().contains("excluyentes"));
        assert!(build_sispro_client(None, Some(plain), Duration::from_secs(1)).is_ok());
        assert!(build_sispro_client(Some(" "), Some(" "), Duration::from_secs(1)).is_ok());
    }
}
