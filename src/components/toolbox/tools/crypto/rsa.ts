import {
  type ByteEncoding,
  CryptoToolError,
  decodeBase64Url,
  decodeBytes,
  encodeBytes,
  encodeUtf8,
  getSubtle,
} from "./encoding"
import {
  formatRsaPrivatePem,
  formatRsaPublicPem,
  rsaPrivateDerFromPem,
  rsaPublicDerFromPem,
} from "./pem"

export type RsaModulus = 2048 | 4096
export type RsaPemFormat = "pkcs1" | "pkcs8"
export type RsaOaepHash = "SHA-1" | "SHA-256"
export type RsaSignScheme = "pkcs1" | "pss"
export type RsaSignHash = "SHA-1" | "SHA-256" | "SHA-384" | "SHA-512"

export interface RsaKeyPairPem {
  publicKey: string
  privateKey: string
}

function asBufferSource(bytes: Uint8Array): ArrayBuffer {
  return bytes.buffer.slice(
    bytes.byteOffset,
    bytes.byteOffset + bytes.byteLength
  ) as ArrayBuffer
}

export async function generateRsaKeyPair(
  modulusLength: RsaModulus,
  pemFormat: RsaPemFormat
): Promise<RsaKeyPairPem> {
  const pair = await getSubtle().generateKey(
    {
      name: "RSA-OAEP",
      modulusLength,
      publicExponent: new Uint8Array([1, 0, 1]),
      hash: "SHA-256",
    },
    true,
    ["encrypt", "decrypt"]
  )
  const spki = new Uint8Array(
    await getSubtle().exportKey("spki", pair.publicKey)
  )
  const pkcs8 = new Uint8Array(
    await getSubtle().exportKey("pkcs8", pair.privateKey)
  )
  return {
    publicKey: formatRsaPublicPem(spki, pemFormat === "pkcs1"),
    privateKey: formatRsaPrivatePem(pkcs8, pemFormat === "pkcs1"),
  }
}

async function importRsaPublic(
  pem: string,
  algorithm: RsaHashedImportParams,
  usages: KeyUsage[]
): Promise<CryptoKey> {
  const der = rsaPublicDerFromPem(pem)
  return getSubtle().importKey(
    "spki",
    asBufferSource(der),
    algorithm,
    false,
    usages
  )
}

async function importRsaPrivate(
  pem: string,
  algorithm: RsaHashedImportParams,
  usages: KeyUsage[]
): Promise<CryptoKey> {
  const der = rsaPrivateDerFromPem(pem)
  return getSubtle().importKey(
    "pkcs8",
    asBufferSource(der),
    algorithm,
    false,
    usages
  )
}

export async function rsaEncrypt(params: {
  publicKeyPem: string
  plaintext: string
  inputEncoding: ByteEncoding
  outputEncoding: ByteEncoding
  hash?: RsaOaepHash
}): Promise<string> {
  const hash = params.hash ?? "SHA-256"
  const key = await importRsaPublic(
    params.publicKeyPem,
    { name: "RSA-OAEP", hash },
    ["encrypt"]
  )
  const data = decodeBytes(params.inputEncoding, params.plaintext)
  try {
    const out = await getSubtle().encrypt({ name: "RSA-OAEP" }, key, data)
    return encodeBytes(params.outputEncoding, new Uint8Array(out))
  } catch {
    throw new CryptoToolError(
      "invalid-input",
      "RSA-OAEP encryption failed. Message may be longer than the key allows."
    )
  }
}

export async function rsaDecrypt(params: {
  privateKeyPem: string
  ciphertext: string
  inputEncoding: ByteEncoding
  outputEncoding: ByteEncoding
  hash?: RsaOaepHash
}): Promise<string> {
  const hash = params.hash ?? "SHA-256"
  const key = await importRsaPrivate(
    params.privateKeyPem,
    { name: "RSA-OAEP", hash },
    ["decrypt"]
  )
  const data = decodeBytes(params.inputEncoding, params.ciphertext)
  try {
    const out = await getSubtle().decrypt({ name: "RSA-OAEP" }, key, data)
    return encodeBytes(params.outputEncoding, new Uint8Array(out))
  } catch {
    throw new CryptoToolError(
      "invalid-input",
      "RSA-OAEP decryption failed. Wrong key, hash, or ciphertext encoding."
    )
  }
}

function decodeSignature(
  signature: string,
  encoding: ByteEncoding | "base64url"
): Uint8Array {
  if (encoding === "base64url") return decodeBase64Url(signature)
  return decodeBytes(encoding, signature)
}

export async function rsaSign(params: {
  privateKeyPem: string
  message: string
  messageEncoding: ByteEncoding
  outputEncoding: ByteEncoding
  scheme: RsaSignScheme
  hash: RsaSignHash
}): Promise<string> {
  const name = params.scheme === "pss" ? "RSA-PSS" : "RSASSA-PKCS1-v1_5"
  const key = await importRsaPrivate(
    params.privateKeyPem,
    { name, hash: params.hash },
    ["sign"]
  )
  const data = decodeBytes(params.messageEncoding, params.message)
  const algo: AlgorithmIdentifier | RsaPssParams =
    params.scheme === "pss"
      ? { name: "RSA-PSS", saltLength: hashSaltLength(params.hash) }
      : { name: "RSASSA-PKCS1-v1_5" }
  const out = await getSubtle().sign(algo, key, data)
  return encodeBytes(params.outputEncoding, new Uint8Array(out))
}

export async function rsaVerify(params: {
  publicKeyPem: string
  message: string
  messageEncoding: ByteEncoding
  signature: string
  signatureEncoding: ByteEncoding | "base64url"
  scheme: RsaSignScheme
  hash: RsaSignHash
}): Promise<boolean> {
  const name = params.scheme === "pss" ? "RSA-PSS" : "RSASSA-PKCS1-v1_5"
  const key = await importRsaPublic(
    params.publicKeyPem,
    { name, hash: params.hash },
    ["verify"]
  )
  const data =
    params.messageEncoding === "utf8"
      ? encodeUtf8(params.message)
      : decodeBytes(params.messageEncoding, params.message)
  const signature = decodeSignature(params.signature, params.signatureEncoding)
  const algo: AlgorithmIdentifier | RsaPssParams =
    params.scheme === "pss"
      ? { name: "RSA-PSS", saltLength: hashSaltLength(params.hash) }
      : { name: "RSASSA-PKCS1-v1_5" }
  return getSubtle().verify(algo, key, signature, data)
}

function hashSaltLength(hash: RsaSignHash): number {
  if (hash === "SHA-1") return 20
  if (hash === "SHA-256") return 32
  if (hash === "SHA-384") return 48
  return 64
}
