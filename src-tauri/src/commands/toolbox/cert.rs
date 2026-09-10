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

    const TEST_CERT: &str = r#"-----BEGIN CERTIFICATE-----
MIIC9zCCAd+gAwIBAgIJAKC3I1OPQBJCMA0GCSqGSIb3DQEBCwUAMCcxFTATBgNV
BAMMDHRvb2xib3gudGVzdDEOMAwGA1UECgwFQ29kZWcwHhcNMjYwOTEwMTI0NjU1
WhcNMzYwOTA3MTI0NjU1WjAnMRUwEwYDVQQDDAx0b29sYm94LnRlc3QxDjAMBgNV
BAoMBUNvZGVnMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA2WPN2I0a
aGBb4QCdairjofYQB7K0xQbXSX/159sCwjVgKzbt/GiHjsDSH6CKazv3fTueufSL
OES7dUsTdSg5JhIQQLo+TqUwFxI4Y4eDWhR8H2Ta1enjUE3pcfr34+4scjDrUK5f
VgvwryGbPDtbMvjUKJ3otZ+dWqKxlhEChV8nX5Sxsvi9qJ2QG6MGZR+ZBhKYrjMk
LlYRunN8GY/zfyStEYSnI6QG8EaJfwxksqXpmnoL0PavUSVP9IlUdIU9Dfhc3e+L
esc2KK2zIAohZcBdJOe/IunwUigKPxgB7SETOX0o2MylNDhWr3NnJs2X2Qz2RnK5
SttS411g7TgviQIDAQABoyYwJDAiBgNVHREEGzAZggx0b29sYm94LnRlc3SCCWxv
Y2FsaG9zdDANBgkqhkiG9w0BAQsFAAOCAQEAxkzCG+Llo2csO8A56LSHvbU/u0Ak
yQTmaHYGlZb/4YZ2JYnEOCfVDZQytuMwzcB4AJ4ZMVdQLc/gQRShMbGZmUT4/nmE
9G1YPHlGkSx2S4u5ZPkqmpYfRd7R4I56XDOIYnnZTvqnxH7sLPtRXycmLXLZdxwx
Ier8VapNO8XGarxlATWTV51K6PWh21+2kmmrxAX1as3jNsFxUbzWc4ni1iC5pBLc
Y+rOpPQdLqYEnr9Rga0fQ5hH6/9AJ2QeaA4wd1a8yKB5oV7kJ0ZaRzfp9E8fn0KO
X2ZAluIspCxdjwES1Av162SJOvMaPm/BltWQ1dp1ywoNd8ghSrABjAP7zA==
-----END CERTIFICATE-----"#;

    #[test]
    fn rejects_garbage() {
        let err = parse_cert_pem("not a cert").unwrap_err();
        let msg = err.message.to_lowercase();
        assert!(
            msg.contains("pem") || msg.contains("certificate") || msg.contains("x.509")
        );
    }

    #[test]
    fn parses_subject_and_san() {
        let view = parse_cert_pem(TEST_CERT).expect("valid test cert");
        assert!(view.subject.contains("toolbox.test"));
        assert!(view.san.iter().any(|n| n.contains("localhost")));
        assert_eq!(view.fingerprint_sha256.len(), 64);
    }
}
