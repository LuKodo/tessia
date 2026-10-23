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

impl PinVerifier {
    /// Checks the handshake signature against the public key of the pinned
    /// certificate. The key is read straight from the DER instead of through
    /// webpki, which refuses the X.509 v1 certificates older servers present.
    fn verify_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        let key = subject_public_key(cert.as_ref()).ok_or_else(|| {
            Error::General("no se pudo leer la clave publica del certificado".into())
        })?;
        let algorithms = self
            .algorithms
            .mapping
            .iter()
            .filter(|(scheme, _)| *scheme == dss.scheme)
            .flat_map(|(_, algorithms)| algorithms.iter());
        for algorithm in algorithms {
            if algorithm
                .verify_signature(key, message, dss.signature())
                .is_ok()
            {
                return Ok(HandshakeSignatureValid::assertion());
            }
        }
        Err(Error::General(
            "la firma del handshake no coincide con la clave del certificado".into(),
        ))
    }
}

/// TLS 1.3 forbids the PKCS#1 v1.5 and SHA-1 signature schemes.
fn tls13_scheme(scheme: SignatureScheme) -> bool {
    !matches!(
        scheme,
        SignatureScheme::RSA_PKCS1_SHA1
            | SignatureScheme::ECDSA_SHA1_Legacy
            | SignatureScheme::RSA_PKCS1_SHA256
            | SignatureScheme::RSA_PKCS1_SHA384
            | SignatureScheme::RSA_PKCS1_SHA512
    )
}

/// One DER element: its tag, its content and what follows it.
fn der_element(bytes: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = bytes.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let count = usize::from(first & 0x7f);
        if count == 0 || count > 4 || rest.len() < count {
            return None;
        }
        let len = rest[..count]
            .iter()
            .fold(0usize, |acc, b| (acc << 8) | usize::from(*b));
        (len, &rest[count..])
    };
    (rest.len() >= len).then(|| (tag, &rest[..len], &rest[len..]))
}

/// The `subjectPublicKey` bits of a certificate (v1 or v3): the key material
/// the signature algorithms verify against.
fn subject_public_key(cert: &[u8]) -> Option<&[u8]> {
    let (_, certificate, _) = der_element(cert)?;
    let (_, mut tbs, _) = der_element(certificate)?;
    let (tag, _, after) = der_element(tbs)?;
    if tag == 0xA0 {
        tbs = after; // explicit [0] version (absent in v1)
    }
    // serial, signature algorithm, issuer, validity, subject
    for _ in 0..5 {
        tbs = der_element(tbs)?.2;
    }
    let (_, spki, _) = der_element(tbs)?;
    let (_, _algorithm, rest) = der_element(spki)?;
    let (tag, bits, _) = der_element(rest)?;
    (tag == 0x03).then(|| bits.split_first().map(|(_, key)| key))?
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
        self.verify_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        if !tls13_scheme(dss.scheme) {
            return Err(Error::General(
                "firma del handshake con un esquema no valido para TLS 1.3".into(),
            ));
        }
        self.verify_signature(message, cert, dss)
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

    fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        if content.len() < 0x80 {
            out.push(content.len() as u8);
        } else {
            out.extend([0x82, (content.len() >> 8) as u8, content.len() as u8]);
        }
        out.extend(content);
        out
    }

    fn certificate(with_version: bool, key: &[u8]) -> Vec<u8> {
        let mut tbs = Vec::new();
        if with_version {
            tbs.extend(tlv(0xA0, &tlv(0x02, &[2])));
        }
        tbs.extend(tlv(0x02, &[1])); // serial
        tbs.extend(tlv(0x30, &[])); // signature algorithm
        tbs.extend(tlv(0x30, &[])); // issuer
        tbs.extend(tlv(0x30, &[])); // validity
        tbs.extend(tlv(0x30, &[])); // subject
        let mut bits = vec![0u8];
        bits.extend(key);
        let mut spki = tlv(0x30, &[]);
        spki.extend(tlv(0x03, &bits));
        tbs.extend(tlv(0x30, &spki));
        let mut cert = tlv(0x30, &tbs);
        cert.extend(tlv(0x30, &[])); // outer algorithm (ignored)
        tlv(0x30, &cert)
    }

    #[test]
    fn the_public_key_is_read_from_v1_and_v3_certificates() {
        let key: Vec<u8> = (0..200u8).collect();
        for with_version in [false, true] {
            let der = certificate(with_version, &key);
            assert_eq!(subject_public_key(&der), Some(key.as_slice()));
        }
        assert_eq!(subject_public_key(&[0x30, 0x02, 0x01]), None);
        assert_eq!(subject_public_key(&[]), None);
    }

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
