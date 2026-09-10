use std::fs::File;
use std::io::{Read, Write};
use std::sync::atomic::AtomicBool;

use aes::{Aes128, Aes192, Aes256};
use aes_gcm::aead::{Aead, KeyInit as AeadKeyInit, Payload};
use aes_gcm::{Aes128Gcm, Aes256Gcm, Nonce};
use cipher::{generic_array::GenericArray, BlockDecrypt, BlockEncrypt};
use serde::Deserialize;

use super::bytes::{decode_bytes, expect_len, ByteEncoding};
use super::jobs::{cancelled_error, emit_progress, is_cancelled};
use crate::app_error::AppCommandError;
use crate::web::event_bridge::EventEmitter;

const CHUNK: usize = 64 * 1024;
const BLOCK: usize = 16;
const GCM_MAX_BYTES: u64 = 64 * 1024 * 1024;
const KDF_MAGIC: &[u8; 6] = b"CGKDF2";

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Algorithm {
    Aes128,
    Aes192,
    Aes256,
    Sm4,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Cbc,
    Gcm,
    Ecb,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Padding {
    Pkcs7,
    Zero,
    None,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Encrypt,
    Decrypt,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CipherFileParams {
    pub src_path: String,
    pub dest_path: String,
    pub algorithm: Algorithm,
    pub mode: Mode,
    pub padding: Padding,
    pub direction: Direction,
    pub key: String,
    pub key_encoding: ByteEncoding,
    pub iv: String,
    pub iv_encoding: ByteEncoding,
    pub prepend_iv: bool,
    pub passphrase: Option<String>,
    pub pbkdf2_iterations: Option<u32>,
    pub job_id: String,
}

fn key_len(alg: Algorithm) -> usize {
    match alg {
        Algorithm::Aes128 | Algorithm::Sm4 => 16,
        Algorithm::Aes192 => 24,
        Algorithm::Aes256 => 32,
    }
}

enum BlockCipher {
    Aes128(Aes128),
    Aes192(Aes192),
    Aes256(Aes256),
    Sm4(sm4::Sm4),
}

impl BlockCipher {
    fn new(alg: Algorithm, key: &[u8]) -> Result<Self, AppCommandError> {
        expect_len(key, key_len(alg), "key-length", "Key")?;
        Ok(match alg {
            Algorithm::Aes128 => Self::Aes128(Aes128::new(GenericArray::from_slice(key))),
            Algorithm::Aes192 => Self::Aes192(Aes192::new(GenericArray::from_slice(key))),
            Algorithm::Aes256 => Self::Aes256(Aes256::new(GenericArray::from_slice(key))),
            Algorithm::Sm4 => Self::Sm4(sm4::Sm4::new(GenericArray::from_slice(key))),
        })
    }

    fn encrypt_block(&self, block: &mut [u8; BLOCK]) {
        let mut ga = GenericArray::clone_from_slice(block);
        match self {
            Self::Aes128(c) => c.encrypt_block(&mut ga),
            Self::Aes192(c) => c.encrypt_block(&mut ga),
            Self::Aes256(c) => c.encrypt_block(&mut ga),
            Self::Sm4(c) => c.encrypt_block(&mut ga),
        }
        block.copy_from_slice(&ga);
    }

    fn decrypt_block(&self, block: &mut [u8; BLOCK]) {
        let mut ga = GenericArray::clone_from_slice(block);
        match self {
            Self::Aes128(c) => c.decrypt_block(&mut ga),
            Self::Aes192(c) => c.decrypt_block(&mut ga),
            Self::Aes256(c) => c.decrypt_block(&mut ga),
            Self::Sm4(c) => c.decrypt_block(&mut ga),
        }
        block.copy_from_slice(&ga);
    }
}

fn xor_in_place(a: &mut [u8], b: &[u8]) {
    for (x, y) in a.iter_mut().zip(b) {
        *x ^= *y;
    }
}

fn pad(data: &[u8], padding: Padding) -> Result<Vec<u8>, AppCommandError> {
    match padding {
        Padding::None => {
            if data.len() % BLOCK != 0 {
                return Err(AppCommandError::invalid_input(format!(
                    "NoPadding requires input length to be a multiple of {BLOCK} bytes (got {}).",
                    data.len()
                )));
            }
            Ok(data.to_vec())
        }
        Padding::Zero => {
            let rem = data.len() % BLOCK;
            if rem == 0 {
                return Ok(data.to_vec());
            }
            let mut out = vec![0u8; data.len() + (BLOCK - rem)];
            out[..data.len()].copy_from_slice(data);
            Ok(out)
        }
        Padding::Pkcs7 => {
            let n = BLOCK - (data.len() % BLOCK);
            let mut out = vec![n as u8; data.len() + n];
            out[..data.len()].copy_from_slice(data);
            Ok(out)
        }
    }
}

fn unpad(data: &[u8], padding: Padding) -> Result<Vec<u8>, AppCommandError> {
    match padding {
        Padding::None => {
            if data.len() % BLOCK != 0 {
                return Err(AppCommandError::invalid_input(format!(
                    "NoPadding requires ciphertext length to be a multiple of {BLOCK} bytes (got {}).",
                    data.len()
                )));
            }
            Ok(data.to_vec())
        }
        Padding::Zero => {
            let mut end = data.len();
            while end > 0 && data[end - 1] == 0 {
                end -= 1;
            }
            Ok(data[..end].to_vec())
        }
        Padding::Pkcs7 => {
            if data.is_empty() || data.len() % BLOCK != 0 {
                return Err(AppCommandError::invalid_input(
                    "PKCS7 padding is invalid: ciphertext length is not a multiple of the block size.",
                ));
            }
            let n = data[data.len() - 1] as usize;
            if n < 1 || n > BLOCK {
                return Err(AppCommandError::invalid_input("PKCS7 padding is invalid."));
            }
            if data[data.len() - n..].iter().any(|&b| b as usize != n) {
                return Err(AppCommandError::invalid_input("PKCS7 padding is invalid."));
            }
            Ok(data[..data.len() - n].to_vec())
        }
    }
}

fn derive_key(passphrase: &str, salt: &[u8], iterations: u32, len: usize) -> Vec<u8> {
    let mut key = vec![0u8; len];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(passphrase.as_bytes(), salt, iterations, &mut key);
    key
}

fn aes_gcm_crypt(
    alg: Algorithm,
    direction: Direction,
    key: &[u8],
    nonce: &[u8],
    data: &[u8],
) -> Result<Vec<u8>, AppCommandError> {
    if nonce.len() != 12 {
        return Err(AppCommandError::invalid_input(
            "IV / nonce for GCM file mode must be 12 bytes.",
        ));
    }
    let nonce = Nonce::from_slice(nonce);
    let payload = Payload {
        msg: data,
        aad: b"",
    };
    let out = match (alg, direction) {
        (Algorithm::Aes128, Direction::Encrypt) => {
            Aes128Gcm::new(GenericArray::from_slice(key)).encrypt(nonce, payload)
        }
        (Algorithm::Aes128, Direction::Decrypt) => {
            Aes128Gcm::new(GenericArray::from_slice(key)).decrypt(nonce, payload)
        }
        (Algorithm::Aes256, Direction::Encrypt) => {
            Aes256Gcm::new(GenericArray::from_slice(key)).encrypt(nonce, payload)
        }
        (Algorithm::Aes256, Direction::Decrypt) => {
            Aes256Gcm::new(GenericArray::from_slice(key)).decrypt(nonce, payload)
        }
        (Algorithm::Aes192, _) => {
            return Err(AppCommandError::invalid_input(
                "AES-192 GCM file encryption is not supported; use AES-128/256 or CBC.",
            ))
        }
        (Algorithm::Sm4, _) => {
            return Err(AppCommandError::invalid_input(
                "SM4 GCM file encryption is not supported; use CBC or the text tool.",
            ))
        }
    };
    out.map_err(|_| {
        AppCommandError::invalid_input("GCM authentication failed: ciphertext or tag is invalid.")
    })
}

pub fn cipher_file_core(
    params: &CipherFileParams,
    emitter: &EventEmitter,
    cancel: &AtomicBool,
) -> Result<(), AppCommandError> {
    if params.src_path == params.dest_path {
        return Err(AppCommandError::invalid_input(
            "Source and destination paths must differ.",
        ));
    }
    let src_meta = std::fs::metadata(&params.src_path).map_err(AppCommandError::io)?;
    if !src_meta.is_file() {
        return Err(AppCommandError::invalid_input(
            "Source is not a regular file.",
        ));
    }
    let total = src_meta.len();
    if params.mode == Mode::Gcm && total > GCM_MAX_BYTES {
        return Err(AppCommandError::invalid_input(format!(
            "GCM file mode is limited to {GCM_MAX_BYTES} bytes; use CBC for larger files."
        )));
    }

    let mut src = File::open(&params.src_path).map_err(AppCommandError::io)?;
    let mut dest = File::create(&params.dest_path).map_err(AppCommandError::io)?;
    emit_progress(emitter, &params.job_id, "cipher", 0, total);

    let key = if let Some(pass) = params
        .passphrase
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let mut salt = [0u8; 16];
        let iterations = if params.direction == Direction::Encrypt {
            let iterations = params.pbkdf2_iterations.unwrap_or(100_000).max(1);
            getrandom_salt(&mut salt)?;
            dest.write_all(KDF_MAGIC).map_err(AppCommandError::io)?;
            dest.write_all(&salt).map_err(AppCommandError::io)?;
            dest.write_all(&iterations.to_be_bytes())
                .map_err(AppCommandError::io)?;
            iterations
        } else {
            let mut magic = [0u8; 6];
            src.read_exact(&mut magic).map_err(AppCommandError::io)?;
            if &magic != KDF_MAGIC {
                return Err(AppCommandError::invalid_input(
                    "File is not a passphrase-derived toolbox ciphertext (missing CGKDF2 header).",
                ));
            }
            src.read_exact(&mut salt).map_err(AppCommandError::io)?;
            let mut iter_buf = [0u8; 4];
            src.read_exact(&mut iter_buf).map_err(AppCommandError::io)?;
            u32::from_be_bytes(iter_buf)
        };
        derive_key(pass, &salt, iterations, key_len(params.algorithm))
    } else {
        let key = decode_bytes(params.key_encoding, &params.key)?;
        expect_len(&key, key_len(params.algorithm), "key-length", "Key")?;
        key
    };
    cipher_after_key(params, emitter, cancel, &key, src, dest, total)
}

fn getrandom_salt(salt: &mut [u8]) -> Result<(), AppCommandError> {
    use rand::RngCore;
    rand::thread_rng().fill_bytes(salt);
    Ok(())
}

fn cipher_after_key(
    params: &CipherFileParams,
    emitter: &EventEmitter,
    cancel: &AtomicBool,
    key: &[u8],
    mut src: File,
    mut dest: File,
    total: u64,
) -> Result<(), AppCommandError> {
    let mut iv = if params.mode == Mode::Ecb {
        Vec::new()
    } else {
        decode_bytes(params.iv_encoding, &params.iv)?
    };
    if params.mode == Mode::Gcm && !iv.is_empty() {
        if iv.len() != 12 && iv.len() != 16 {
            return Err(AppCommandError::invalid_input(
                "IV / nonce for GCM must be 12 or 16 bytes.",
            ));
        }
    } else if params.mode == Mode::Cbc {
        expect_len(&iv, 16, "iv-length", "IV for CBC")?;
    }

    if params.mode == Mode::Gcm {
        let mut data = Vec::new();
        src.read_to_end(&mut data).map_err(AppCommandError::io)?;
        if is_cancelled(cancel) {
            return Err(cancelled_error());
        }
        let mut body = data;
        if params.direction == Direction::Decrypt && params.prepend_iv {
            let nlen = if iv.len() == 16 { 16 } else { 12 };
            if body.len() < nlen {
                return Err(AppCommandError::invalid_input(
                    "Ciphertext is shorter than the IV prefix.",
                ));
            }
            iv = body[..nlen].to_vec();
            body = body[nlen..].to_vec();
        }
        if params.direction == Direction::Encrypt && params.prepend_iv {
            dest.write_all(&iv).map_err(AppCommandError::io)?;
        }
        if iv.len() == 16 {
            return Err(AppCommandError::invalid_input(
                "GCM file mode supports a 12-byte nonce only.",
            ));
        }
        let out = aes_gcm_crypt(params.algorithm, params.direction, key, &iv, &body)?;
        dest.write_all(&out).map_err(AppCommandError::io)?;
        emit_progress(emitter, &params.job_id, "cipher", total, total);
        return Ok(());
    }

    let cipher = BlockCipher::new(params.algorithm, key)?;
    let mut chain = [0u8; BLOCK];
    if params.mode == Mode::Cbc {
        chain.copy_from_slice(&iv);
        if params.direction == Direction::Encrypt && params.prepend_iv {
            dest.write_all(&iv).map_err(AppCommandError::io)?;
        }
        if params.direction == Direction::Decrypt && params.prepend_iv {
            src.read_exact(&mut chain).map_err(AppCommandError::io)?;
        }
    }

    match params.direction {
        Direction::Encrypt => encrypt_stream(
            params, emitter, cancel, &cipher, &mut chain, &mut src, &mut dest, total,
        ),
        Direction::Decrypt => decrypt_stream(
            params, emitter, cancel, &cipher, &mut chain, &mut src, &mut dest, total,
        ),
    }
}

fn encrypt_block_mode(
    cipher: &BlockCipher,
    mode: Mode,
    chain: &mut [u8; BLOCK],
    block: &mut [u8; BLOCK],
) {
    if mode == Mode::Cbc {
        xor_in_place(block, chain);
    }
    cipher.encrypt_block(block);
    if mode == Mode::Cbc {
        chain.copy_from_slice(block);
    }
}

fn decrypt_block_mode(
    cipher: &BlockCipher,
    mode: Mode,
    chain: &mut [u8; BLOCK],
    block: &mut [u8; BLOCK],
) {
    let incoming = *block;
    cipher.decrypt_block(block);
    if mode == Mode::Cbc {
        xor_in_place(block, chain);
        chain.copy_from_slice(&incoming);
    }
}

fn encrypt_stream(
    params: &CipherFileParams,
    emitter: &EventEmitter,
    cancel: &AtomicBool,
    cipher: &BlockCipher,
    chain: &mut [u8; BLOCK],
    src: &mut File,
    dest: &mut File,
    total: u64,
) -> Result<(), AppCommandError> {
    let mut buf = vec![0u8; CHUNK];
    let mut leftover = Vec::new();
    let mut done = 0u64;
    loop {
        if is_cancelled(cancel) {
            return Err(cancelled_error());
        }
        let n = src.read(&mut buf).map_err(AppCommandError::io)?;
        if n == 0 {
            break;
        }
        leftover.extend_from_slice(&buf[..n]);
        done += n as u64;
        while leftover.len() >= BLOCK {
            let mut block = [0u8; BLOCK];
            block.copy_from_slice(&leftover[..BLOCK]);
            leftover.drain(..BLOCK);
            encrypt_block_mode(cipher, params.mode, chain, &mut block);
            dest.write_all(&block).map_err(AppCommandError::io)?;
        }
        emit_progress(emitter, &params.job_id, "cipher", done, total);
    }
    let padded = pad(&leftover, params.padding)?;
    for chunk in padded.chunks(BLOCK) {
        let mut block = [0u8; BLOCK];
        block.copy_from_slice(chunk);
        encrypt_block_mode(cipher, params.mode, chain, &mut block);
        dest.write_all(&block).map_err(AppCommandError::io)?;
    }
    Ok(())
}

fn decrypt_stream(
    params: &CipherFileParams,
    emitter: &EventEmitter,
    cancel: &AtomicBool,
    cipher: &BlockCipher,
    chain: &mut [u8; BLOCK],
    src: &mut File,
    dest: &mut File,
    total: u64,
) -> Result<(), AppCommandError> {
    let mut buf = vec![0u8; CHUNK];
    let mut leftover = Vec::new();
    let mut pending = Vec::new();
    let mut done = 0u64;
    loop {
        if is_cancelled(cancel) {
            return Err(cancelled_error());
        }
        let n = src.read(&mut buf).map_err(AppCommandError::io)?;
        if n == 0 {
            break;
        }
        leftover.extend_from_slice(&buf[..n]);
        done += n as u64;
        while leftover.len() >= BLOCK {
            let mut block = [0u8; BLOCK];
            block.copy_from_slice(&leftover[..BLOCK]);
            leftover.drain(..BLOCK);
            decrypt_block_mode(cipher, params.mode, chain, &mut block);
            pending.extend_from_slice(&block);
            if pending.len() > BLOCK {
                let flush = pending.len() - BLOCK;
                dest.write_all(&pending[..flush])
                    .map_err(AppCommandError::io)?;
                pending.drain(..flush);
            }
        }
        emit_progress(emitter, &params.job_id, "cipher", done, total);
    }
    if !leftover.is_empty() {
        return Err(AppCommandError::invalid_input(
            "PKCS7 padding is invalid: ciphertext length is not a multiple of the block size.",
        ));
    }
    let plain = unpad(&pending, params.padding)?;
    dest.write_all(&plain).map_err(AppCommandError::io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::event_bridge::EventEmitter;
    use std::sync::atomic::AtomicBool;

    fn roundtrip(alg: Algorithm, mode: Mode, padding: Padding) {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("plain.bin");
        let enc = dir.path().join("enc.bin");
        let out = dir.path().join("out.bin");
        let plain = b"Hello, Codeg file cipher!!".to_vec();
        std::fs::write(&src, &plain).unwrap();
        let key = "1234567890123456";
        let iv = "1234567890123456";
        let job = "c1".to_string();
        let enc_params = CipherFileParams {
            src_path: src.to_string_lossy().into(),
            dest_path: enc.to_string_lossy().into(),
            algorithm: alg,
            mode,
            padding,
            direction: Direction::Encrypt,
            key: key.into(),
            key_encoding: ByteEncoding::Utf8,
            iv: iv.into(),
            iv_encoding: ByteEncoding::Utf8,
            prepend_iv: true,
            passphrase: None,
            pbkdf2_iterations: None,
            job_id: job.clone(),
        };
        let cancel = AtomicBool::new(false);
        cipher_file_core(&enc_params, &EventEmitter::Noop, &cancel).unwrap();
        let dec_params = CipherFileParams {
            src_path: enc.to_string_lossy().into(),
            dest_path: out.to_string_lossy().into(),
            direction: Direction::Decrypt,
            ..enc_params
        };
        cipher_file_core(&dec_params, &EventEmitter::Noop, &cancel).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), plain);
    }

    #[test]
    fn aes128_cbc_pkcs7_file_roundtrip() {
        roundtrip(Algorithm::Aes128, Mode::Cbc, Padding::Pkcs7);
    }

    #[test]
    fn sm4_cbc_pkcs7_file_roundtrip() {
        roundtrip(Algorithm::Sm4, Mode::Cbc, Padding::Pkcs7);
    }

    #[test]
    fn aes128_gcm_file_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("plain.bin");
        let enc = dir.path().join("enc.bin");
        let out = dir.path().join("out.bin");
        std::fs::write(&src, b"gcm-file").unwrap();
        let params = CipherFileParams {
            src_path: src.to_string_lossy().into(),
            dest_path: enc.to_string_lossy().into(),
            algorithm: Algorithm::Aes128,
            mode: Mode::Gcm,
            padding: Padding::None,
            direction: Direction::Encrypt,
            key: "1234567890123456".into(),
            key_encoding: ByteEncoding::Utf8,
            iv: "123456789012".into(),
            iv_encoding: ByteEncoding::Utf8,
            prepend_iv: true,
            passphrase: None,
            pbkdf2_iterations: None,
            job_id: "g".into(),
        };
        let cancel = AtomicBool::new(false);
        cipher_file_core(&params, &EventEmitter::Noop, &cancel).unwrap();
        let dec = CipherFileParams {
            src_path: enc.to_string_lossy().into(),
            dest_path: out.to_string_lossy().into(),
            direction: Direction::Decrypt,
            ..params
        };
        cipher_file_core(&dec, &EventEmitter::Noop, &cancel).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"gcm-file");
    }
}
