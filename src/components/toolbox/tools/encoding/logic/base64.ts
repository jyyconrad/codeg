import {
  binaryStringToBytes,
  bytesToBinaryString,
  utf8Decode,
  utf8Encode,
} from "./bytes"
import { EncodingError } from "./error"

const BASE64_CHAR = /[A-Za-z0-9+/]/

export function encodeBase64Bytes(bytes: Uint8Array): string {
  if (bytes.length === 0) return ""
  return btoa(bytesToBinaryString(bytes))
}

export function decodeBase64Bytes(input: string): Uint8Array {
  const compact = input.replace(/\s+/g, "")
  if (compact === "") return new Uint8Array()
  if (!isLegalBase64(compact)) {
    throw new EncodingError("Invalid Base64")
  }
  const padded = padBase64(compact)
  try {
    return binaryStringToBytes(atob(padded))
  } catch {
    throw new EncodingError("Invalid Base64")
  }
}

export function encodeBase64(text: string): string {
  if (text === "") return ""
  return encodeBase64Bytes(utf8Encode(text))
}

export function decodeBase64(text: string): string {
  const bytes = decodeBase64Bytes(text)
  if (bytes.length === 0) return ""
  return utf8Decode(bytes)
}

function isLegalBase64(value: string): boolean {
  let padding = 0
  for (let i = 0; i < value.length; i++) {
    const ch = value[i]
    if (ch === "=") {
      padding += 1
      if (padding > 2) return false
      continue
    }
    if (padding > 0 || !ch || !BASE64_CHAR.test(ch)) return false
  }
  return true
}

function padBase64(value: string): string {
  const remainder = value.length % 4
  if (remainder === 1) {
    throw new EncodingError("Invalid Base64")
  }
  if (remainder === 0) return value
  return value + "=".repeat(4 - remainder)
}
