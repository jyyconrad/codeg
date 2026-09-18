use crate::app_error::AppCommandError;

pub fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

pub fn decode_hex(input: &str) -> Result<Vec<u8>, AppCommandError> {
    let mut hex: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ':')
        .collect();
    if hex.starts_with("0x") || hex.starts_with("0X") {
        hex = hex[2..].to_string();
    }
    if hex.is_empty() {
        return Ok(Vec::new());
    }
    if !hex.len().is_multiple_of(2) || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppCommandError::invalid_input("Value is not valid hex."));
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|_| AppCommandError::invalid_input("Value is not valid hex."))
        })
        .collect()
}

pub fn decode_base64(input: &str) -> Result<Vec<u8>, AppCommandError> {
    use base64::Engine;
    let compact: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.is_empty() {
        return Ok(Vec::new());
    }
    let mut padded = compact;
    let rem = padded.len() % 4;
    if rem != 0 {
        padded.extend(std::iter::repeat_n('=', 4 - rem));
    }
    base64::engine::general_purpose::STANDARD
        .decode(padded.as_bytes())
        .map_err(|_| AppCommandError::invalid_input("Value is not valid Base64."))
}

#[derive(Debug, Clone, Copy, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ByteEncoding {
    Utf8,
    Hex,
    Base64,
}

pub fn decode_bytes(encoding: ByteEncoding, input: &str) -> Result<Vec<u8>, AppCommandError> {
    match encoding {
        ByteEncoding::Utf8 => Ok(input.as_bytes().to_vec()),
        ByteEncoding::Hex => decode_hex(input),
        ByteEncoding::Base64 => decode_base64(input),
    }
}

pub fn expect_len(
    bytes: &[u8],
    expected: usize,
    kind: &str,
    label: &str,
) -> Result<(), AppCommandError> {
    if bytes.len() == expected {
        return Ok(());
    }
    Err(AppCommandError::invalid_input(format!(
        "{label} must be {expected} bytes ({kind}), got {}.",
        bytes.len()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let raw = vec![0xde, 0xad, 0xbe, 0xef];
        assert_eq!(decode_hex(&encode_hex(&raw)).unwrap(), raw);
        assert_eq!(decode_hex("0xDEAD beef").unwrap(), raw);
    }
}
