import { sm3 } from "sm-crypto-v2"
import {
  type ByteEncoding,
  cloneBytes,
  decodeBytes,
  encodeBase64,
  encodeHex,
  getSubtle,
} from "./encoding"

export type HmacAlgorithm = "SHA-256" | "SHA-512" | "SM3"
export type HmacOutputEncoding = "hex" | "base64"

export async function hmacBytes(
  algorithm: HmacAlgorithm,
  key: Uint8Array,
  message: Uint8Array
): Promise<Uint8Array> {
  if (algorithm === "SM3") {
    const hex = sm3(cloneBytes(message), { key: cloneBytes(key), mode: "hmac" })
    const out = new Uint8Array(hex.length / 2)
    for (let i = 0; i < out.length; i += 1) {
      out[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16)
    }
    return out
  }
  const cryptoKey = await getSubtle().importKey(
    "raw",
    key,
    { name: "HMAC", hash: algorithm },
    false,
    ["sign"]
  )
  const sig = await getSubtle().sign("HMAC", cryptoKey, message)
  return new Uint8Array(sig)
}

export async function hmacText(params: {
  algorithm: HmacAlgorithm
  key: string
  keyEncoding: ByteEncoding
  message: string
  messageEncoding: ByteEncoding
  outputEncoding: HmacOutputEncoding
}): Promise<string> {
  const key = decodeBytes(params.keyEncoding, params.key)
  const message = decodeBytes(params.messageEncoding, params.message)
  const mac = await hmacBytes(params.algorithm, key, message)
  return params.outputEncoding === "hex" ? encodeHex(mac) : encodeBase64(mac)
}
