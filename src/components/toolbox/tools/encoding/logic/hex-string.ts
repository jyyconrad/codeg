import { latin1Decode, latin1Encode, utf8Decode, utf8Encode } from "./bytes"
import { EncodingError } from "./error"

export type HexCharset = "utf-8" | "latin1"

export function encodeHex(text: string, charset: HexCharset): string {
  if (text === "") return ""
  const bytes = charset === "utf-8" ? utf8Encode(text) : latin1Encode(text)
  return bytesToHex(bytes)
}

export function decodeHex(text: string, charset: HexCharset): string {
  const bytes = hexToBytes(text)
  if (bytes.length === 0) return ""
  return charset === "utf-8" ? utf8Decode(bytes) : latin1Decode(bytes)
}

export function bytesToHex(bytes: Uint8Array): string {
  let hex = ""
  for (let i = 0; i < bytes.length; i++) {
    hex += (bytes[i] ?? 0).toString(16).padStart(2, "0")
  }
  return hex
}

export function hexToBytes(input: string): Uint8Array {
  const hex = normalizeHex(input)
  if (hex === "") return new Uint8Array()
  if (hex.length % 2 !== 0 || /[^0-9a-fA-F]/.test(hex)) {
    throw new EncodingError("Invalid hex string")
  }
  const bytes = new Uint8Array(hex.length / 2)
  for (let i = 0; i < bytes.length; i++) {
    bytes[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16)
  }
  return bytes
}

function normalizeHex(input: string): string {
  const tokens = input.trim().split(/\s+/)
  let hex = ""
  for (const token of tokens) {
    if (!token) continue
    hex += token.replace(/^0x/i, "")
  }
  return hex
}
