import { sm2 } from "sm-crypto-v2"
import {
  type ByteEncoding,
  CryptoToolError,
  cloneBytes,
  decodeBytes,
  decodeHex,
  encodeBytes,
  encodeHex,
} from "./encoding"

export type Sm2CipherMode = "c1c3c2" | "c1c2c3"

let rngReady: Promise<void> | null = null

function ensureRng(): Promise<void> {
  rngReady ??= sm2.initRNGPool()
  return rngReady
}

function cipherModeFlag(mode: Sm2CipherMode): number {
  return mode === "c1c2c3" ? 0 : 1
}

export function normalizeSm2PublicKey(input: string): string {
  const hex = encodeHex(decodeHex(input)).toLowerCase()
  if (hex.length === 130 && hex.startsWith("04")) return hex
  if (hex.length === 128) return `04${hex}`
  if (hex.length === 66 && (hex.startsWith("02") || hex.startsWith("03"))) {
    return hex
  }
  throw new CryptoToolError(
    "invalid-input",
    "SM2 public key must be uncompressed (04 + 64 bytes) or compressed hex."
  )
}

export function formatSm2PublicKey(
  publicKey: string,
  includeUncompressedPrefix: boolean
): string {
  const normalized = normalizeSm2PublicKey(publicKey)
  if (!includeUncompressedPrefix && normalized.startsWith("04")) {
    return normalized.slice(2)
  }
  return normalized
}

export function normalizeSm2PrivateKey(input: string): string {
  const hex = encodeHex(decodeHex(input)).toLowerCase()
  if (hex.length !== 64) {
    throw new CryptoToolError(
      "key-length",
      `SM2 private key must be 32 bytes (got ${hex.length / 2}).`
    )
  }
  return hex
}

export async function generateSm2KeyPair(
  includeUncompressedPrefix: boolean
): Promise<{ publicKey: string; privateKey: string }> {
  await ensureRng()
  const pair = sm2.generateKeyPairHex()
  return {
    publicKey: formatSm2PublicKey(pair.publicKey, includeUncompressedPrefix),
    privateKey: pair.privateKey,
  }
}

export async function sm2Encrypt(params: {
  publicKey: string
  plaintext: string
  inputEncoding: ByteEncoding
  outputEncoding: ByteEncoding
  cipherMode: Sm2CipherMode
  asn1: boolean
}): Promise<string> {
  await ensureRng()
  const publicKey = normalizeSm2PublicKey(params.publicKey)
  const plain = decodeBytes(params.inputEncoding, params.plaintext)
  const hex = sm2.doEncrypt(
    cloneBytes(plain),
    publicKey,
    cipherModeFlag(params.cipherMode),
    { asn1: params.asn1 }
  )
  if (params.outputEncoding === "hex") return hex
  return encodeBytes(params.outputEncoding, decodeHex(hex))
}

export async function sm2Decrypt(params: {
  privateKey: string
  ciphertext: string
  inputEncoding: ByteEncoding
  outputEncoding: ByteEncoding
  cipherMode: Sm2CipherMode
  asn1: boolean
}): Promise<string> {
  const privateKey = normalizeSm2PrivateKey(params.privateKey)
  const cipherHex =
    params.inputEncoding === "hex"
      ? encodeHex(decodeHex(params.ciphertext))
      : encodeHex(decodeBytes(params.inputEncoding, params.ciphertext))
  const out = sm2.doDecrypt(
    cipherHex,
    privateKey,
    cipherModeFlag(params.cipherMode),
    { output: "array", asn1: params.asn1 }
  )
  // sm-crypto-v2 returns a plain [] on C3 mismatch, vs Uint8Array on success.
  if (Array.isArray(out) || !(out instanceof Uint8Array)) {
    throw new CryptoToolError(
      "invalid-input",
      "SM2 decryption failed: C3 checksum mismatch or wrong key/mode."
    )
  }
  return encodeBytes(params.outputEncoding, out)
}

export async function sm2Sign(params: {
  privateKey: string
  publicKey?: string
  message: string
  messageEncoding: ByteEncoding
  der: boolean
}): Promise<string> {
  await ensureRng()
  const privateKey = normalizeSm2PrivateKey(params.privateKey)
  const msg = cloneBytes(decodeBytes(params.messageEncoding, params.message))
  return sm2.doSignature(msg, privateKey, {
    hash: true,
    der: params.der,
    publicKey: params.publicKey
      ? normalizeSm2PublicKey(params.publicKey)
      : undefined,
  })
}

export function sm2Verify(params: {
  publicKey: string
  message: string
  messageEncoding: ByteEncoding
  signature: string
  der: boolean
}): boolean {
  const publicKey = normalizeSm2PublicKey(params.publicKey)
  const msg = cloneBytes(decodeBytes(params.messageEncoding, params.message))
  return sm2.doVerifySignature(msg, params.signature.trim(), publicKey, {
    hash: true,
    der: params.der,
  })
}
