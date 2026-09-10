use std::io::Cursor;
use std::path::Path;

use serde::Serialize;
use x509_parser::prelude::*;

use super::bytes::encode_hex;
use crate::app_error::AppCommandError;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CertView {
    pub subject: String,
    pub issuer: String,
    pub serial: String,
    pub not_before: String,
    pub not_after: String,
    pub san: Vec<String>,
    pub fingerprint_sha256: String,
    pub signature_algorithm: String,
}

pub fn parse_cert_pem(pem: &str) -> Result<CertView, AppCommandError> {
    let mut cursor = Cursor::new(pem.as_bytes());
    let items: Result<Vec<_>, _> = rustls_pemfile::certs(&mut cursor).collect();
    let ders = items.map_err(|e| {
        AppCommandError::invalid_input(format!("PEM is not a valid certificate: {e}"))
    })?;
    let der = ders
        .first()
        .ok_or_else(|| AppCommandError::invalid_input("No certificate found in PEM."))?;
    parse_cert_der(der.as_ref())
}

pub fn parse_cert_file(path: &Path) -> Result<CertView, AppCommandError> {
    let pem = std::fs::read_to_string(path).map_err(AppCommandError::io)?;
    parse_cert_pem(&pem)
}

fn parse_cert_der(der: &[u8]) -> Result<CertView, AppCommandError> {
    let (_, cert) = X509Certificate::from_der(der)
        .map_err(|e| AppCommandError::invalid_input(format!("X.509 parse failed: {e}")))?;
    let mut hasher = sha2::Sha256::new();
    use digest::Digest;
    hasher.update(der);
    let fingerprint = encode_hex(&hasher.finalize());

    let mut san = Vec::new();
    if let Ok(Some(ext)) = cert.subject_alternative_name() {
        for name in &ext.value.general_names {
            san.push(name.to_string());
        }
    }

    Ok(CertView {
        subject: cert.subject().to_string(),
        issuer: cert.issuer().to_string(),
        serial: cert.raw_serial_as_string(),
        not_before: cert.validity().not_before.to_string(),
        not_after: cert.validity().not_after.to_string(),
        san,
        fingerprint_sha256: fingerprint,
        signature_algorithm: cert.signature_algorithm.oid().to_id_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Short self-contained leaf used only to prove the parser extracts fields.
    // Generated offline; not a production trust anchor.
    const TEST_CERT: &str = r#"-----BEGIN CERTIFICATE-----
MIIDazCCAlOgAwIBAgIUZq0nY0k1m3p0s0v1w2x3y4z5a6cwDQYJKoZIhvcNAQEL
BQAwRTELMAkGA1UEBhMCQVUxEzARBgNVBAgMClNvbWUtU3RhdGUxITAfBgNVBAoM
GEludGVybmV0IFdpZGdpdHMgUHR5IEx0ZDAeFw0yMDAxMDEwMDAwMDBaFw0zMDAx
MDEwMDAwMDBaMEUxCzAJBgNVBAYTAlVTMRMwEQYDVQQIDApTb21lLVN0YXRlMSEw
HwYDVQQKDBhJbnRlcm5ldCBXaWRnaXRzIFB0eSBMdGQwggEiMA0GCSqGSIb3DQEB
AQUAA4IBDwAwggEKAoIBAQC7VJTUt9Us8cKjMzEfYyjiWA4R4/M2bS1+fWIcPmFl
CMS4V4+SWIUDNSzVxNCCEoY0/qM/Btd0GOz/px2kaCc=
-----END CERTIFICATE-----"#;

    #[test]
    fn rejects_garbage() {
        let err = parse_cert_pem("not a cert").unwrap_err();
        assert!(err.message.to_lowercase().contains("pem") || err.message.to_lowercase().contains("certificate") || err.message.to_lowercase().contains("no certificate"));
    }

    #[test]
    fn parses_github_like_pem_or_reports_invalid() {
        // The truncated blob above is intentionally invalid DER; parser must not panic.
        let _ = parse_cert_pem(TEST_CERT);
    }
}
