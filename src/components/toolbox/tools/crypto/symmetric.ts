import CryptoJS from "crypto-js"
import { sm4 } from "sm-crypto-v2"
import {
  type ByteEncoding,
  CryptoToolError,
  cloneBytes,
  concatBytes,
  decodeBytes,
  encodeBytes,
  expectLength,
  expectOneOfLengths,
  getSubtle,
  randomBytes,
} from "./encoding"
import { type PaddingMode, padBytes, unpadBytes } from "./padding"
import { bytesToWordArray, wordArrayToBytes } from "./word-array"

export type SymmetricAlgorithm = "aes-128" | "aes-192" | "aes-256" | "sm4"
export type CipherMode = "cbc" | "gcm" | "ecb"
export type CipherDirection = "encrypt" | "decrypt"

export const KEY_BYTES: Record<SymmetricAlgorithm, number> = {
  "aes-128": 16,
  "aes-192": 24,
  "aes-256": 32,
  sm4: 16,
}

export function ivLengthFor(mode: CipherMode): number | 0 {
  if (mode === "ecb") return 0
  if (mode === "gcm") return 12
  return 16
}

export function generateIv(mode: CipherMode, encoding: ByteEncoding): string {
  const length = mode === "gcm" ? 12 : 16
  return encodeBytes(
    encoding === "utf8" ? "hex" : encoding,
    randomBytes(length)
  )
}

export interface SymmetricParams {
  algorithm: SymmetricAlgorithm
  mode: CipherMode
  padding: PaddingMode
  direction: CipherDirection
  key: string
  keyEncoding: ByteEncoding
  iv: string
  ivEncoding: ByteEncoding
  input: string
  inputEncoding: ByteEncoding
  outputEncoding: ByteEncoding
  prependIv: boolean
}

function decodeKey(params: SymmetricParams): Uint8Array {
  const key = decodeBytes(params.keyEncoding, params.key)
  expectLength(
    key,
    KEY_BYTES[params.algorithm],
    "key-length",
    `Key for ${params.algorithm.toUpperCase()}`
  )
  return key
}

function decodeIv(params: SymmetricParams, forDecrypt: boolean): Uint8Array {
  if (params.mode === "ecb") return new Uint8Array()
  const expected = ivLengthFor(params.mode)
  if (forDecrypt && params.prependIv) return new Uint8Array()
  const iv = decodeBytes(params.ivEncoding, params.iv)
  if (params.mode === "gcm") {
    expectOneOfLengths(iv, [12, 16], "iv-length", "IV / nonce for GCM")
    return iv
  }
  expectLength(iv, expected, "iv-length", "IV for CBC")
  return iv
}

function aesCbcEcb(
  direction: CipherDirection,
  key: Uint8Array,
  iv: Uint8Array,
  data: Uint8Array,
  mode: "cbc" | "ecb"
): Uint8Array {
  const keyWA = bytesToWordArray(key)
  const dataWA = bytesToWordArray(data)
  const cfg = {
    mode: mode === "cbc" ? CryptoJS.mode.CBC : CryptoJS.mode.ECB,
    padding: CryptoJS.pad.NoPadding,
    ...(mode === "cbc" ? { iv: bytesToWordArray(iv) } : {}),
  }
  if (direction === "encrypt") {
    const encrypted = CryptoJS.AES.encrypt(dataWA, keyWA, cfg)
    return wordArrayToBytes(encrypted.ciphertext)
  }
  const cipherParams = CryptoJS.lib.CipherParams.create({ ciphertext: dataWA })
  const decrypted = CryptoJS.AES.decrypt(cipherParams, keyWA, cfg)
  return wordArrayToBytes(decrypted)
}

async function aesGcm(
  direction: CipherDirection,
  key: Uint8Array,
  iv: Uint8Array,
  data: Uint8Array
): Promise<Uint8Array> {
  const subtle = getSubtle()
  const cryptoKey = await subtle.importKey(
    "raw",
    key,
    { name: "AES-GCM" },
    false,
    direction === "encrypt" ? ["encrypt"] : ["decrypt"]
  )
  try {
    if (direction === "encrypt") {
      const out = await subtle.encrypt(
        { name: "AES-GCM", iv, tagLength: 128 },
        cryptoKey,
        data
      )
      return new Uint8Array(out)
    }
    const out = await subtle.decrypt(
      { name: "AES-GCM", iv, tagLength: 128 },
      cryptoKey,
      data
    )
    return new Uint8Array(out)
  } catch (err) {
    if (direction === "decrypt") {
      throw new CryptoToolError(
        "gcm-auth",
        "GCM authentication failed: ciphertext or tag is invalid."
      )
    }
    throw err
  }
}

function sm4Block(
  direction: CipherDirection,
  key: Uint8Array,
  iv: Uint8Array,
  data: Uint8Array,
  mode: "cbc" | "ecb"
): Uint8Array {
  const options = {
    padding: "none" as const,
    mode,
    iv: mode === "cbc" ? cloneBytes(iv) : undefined,
    output: "array" as const,
  }
  const payload = cloneBytes(data)
  const rawKey = cloneBytes(key)
  const out =
    direction === "encrypt"
      ? sm4.encrypt(payload, rawKey, options)
      : sm4.decrypt(payload, rawKey, options)
  if (!(out instanceof Uint8Array)) {
    throw new CryptoToolError("unsupported", "Unexpected SM4 output type.")
  }
  return out
}

function sm4Gcm(
  direction: CipherDirection,
  key: Uint8Array,
  iv: Uint8Array,
  data: Uint8Array
): Uint8Array {
  try {
    if (direction === "encrypt") {
      const result = sm4.encrypt(cloneBytes(data), cloneBytes(key), {
        mode: "gcm",
        iv: cloneBytes(iv),
        output: "array",
        outputTag: true,
      })
      if (!("output" in result) || !(result.output instanceof Uint8Array)) {
        throw new CryptoToolError("unsupported", "Unexpected SM4 GCM output.")
      }
      const tag = result.tag
      if (!(tag instanceof Uint8Array)) {
        throw new CryptoToolError("unsupported", "SM4 GCM tag missing.")
      }
      return concatBytes(result.output, tag)
    }
    if (data.length < 16) {
      throw new CryptoToolError(
        "gcm-auth",
        "GCM authentication failed: ciphertext or tag is invalid."
      )
    }
    const ciphertext = data.subarray(0, data.length - 16)
    const tag = data.subarray(data.length - 16)
    const out = sm4.decrypt(cloneBytes(ciphertext), cloneBytes(key), {
      mode: "gcm",
      iv: cloneBytes(iv),
      tag: cloneBytes(tag),
      output: "array",
    })
    if (!(out instanceof Uint8Array)) {
      throw new CryptoToolError("unsupported", "Unexpected SM4 GCM output.")
    }
    return out
  } catch (err) {
    if (err instanceof CryptoToolError) throw err
    throw new CryptoToolError(
      "gcm-auth",
      "GCM authentication failed: ciphertext or tag is invalid."
    )
  }
}

function splitPrependedIv(
  packed: Uint8Array,
  ivLen: number
): { iv: Uint8Array; body: Uint8Array } {
  if (packed.length < ivLen) {
    throw new CryptoToolError(
      "iv-length",
      `Ciphertext is shorter than the ${ivLen}-byte IV prefix (got ${packed.length}).`
    )
  }
  return { iv: packed.subarray(0, ivLen), body: packed.subarray(ivLen) }
}

export async function runSymmetric(params: SymmetricParams): Promise<string> {
  const key = decodeKey(params)
  const usesIv = params.mode !== "ecb"
  const padding: PaddingMode = params.mode === "gcm" ? "none" : params.padding

  if (params.direction === "encrypt") {
    const iv = decodeIv(params, false)
    const plain = decodeBytes(params.inputEncoding, params.input)
    const toEncrypt = params.mode === "gcm" ? plain : padBytes(plain, padding)
    let body: Uint8Array
    if (params.algorithm === "sm4") {
      body =
        params.mode === "gcm"
          ? sm4Gcm("encrypt", key, iv, toEncrypt)
          : sm4Block("encrypt", key, iv, toEncrypt, params.mode)
    } else if (params.mode === "gcm") {
      body = await aesGcm("encrypt", key, iv, toEncrypt)
    } else {
      body = aesCbcEcb("encrypt", key, iv, toEncrypt, params.mode)
    }
    const packed = usesIv && params.prependIv ? concatBytes(iv, body) : body
    return encodeBytes(params.outputEncoding, packed)
  }

  const packed = decodeBytes(params.inputEncoding, params.input)
  let iv = decodeIv(params, true)
  let body = packed
  if (usesIv && params.prependIv) {
    let ivLen = params.mode === "gcm" ? 12 : 16
    if (params.mode === "gcm" && params.iv.trim()) {
      const hinted = decodeBytes(params.ivEncoding, params.iv)
      if (hinted.length === 16) ivLen = 16
    }
    const split = splitPrependedIv(packed, ivLen)
    iv = split.iv
    body = split.body
    if (params.mode === "gcm") {
      expectOneOfLengths(iv, [12, 16], "iv-length", "IV / nonce for GCM")
    } else {
      expectLength(iv, 16, "iv-length", "IV for CBC")
    }
  } else if (usesIv) {
    iv = decodeIv(params, false)
  }

  let decrypted: Uint8Array
  try {
    if (params.algorithm === "sm4") {
      decrypted =
        params.mode === "gcm"
          ? sm4Gcm("decrypt", key, iv, body)
          : sm4Block("decrypt", key, iv, body, params.mode)
    } else if (params.mode === "gcm") {
      decrypted = await aesGcm("decrypt", key, iv, body)
    } else {
      decrypted = aesCbcEcb("decrypt", key, iv, body, params.mode)
    }
  } catch (err) {
    if (err instanceof CryptoToolError) throw err
    if (params.mode !== "gcm") {
      throw new CryptoToolError("bad-padding", "PKCS7 padding is invalid.")
    }
    throw err
  }
  const plain =
    params.mode === "gcm" ? decrypted : unpadBytes(decrypted, padding)
  return encodeBytes(params.outputEncoding, plain)
}
