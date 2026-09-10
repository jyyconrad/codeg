import { EncodingError } from "./error"

const BINARY_CHUNK = 0x8000

export function utf8Encode(text: string): Uint8Array {
  return new TextEncoder().encode(text)
}

export function utf8Decode(bytes: Uint8Array): string {
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes)
  } catch {
    throw new EncodingError("Invalid UTF-8")
  }
}

export function latin1Encode(text: string): Uint8Array {
  const bytes = new Uint8Array(text.length)
  for (let i = 0; i < text.length; i++) {
    const code = text.charCodeAt(i)
    if (code > 255) {
      throw new EncodingError("Character cannot be encoded as Latin-1")
    }
    bytes[i] = code
  }
  return bytes
}

export function latin1Decode(bytes: Uint8Array): string {
  let out = ""
  for (let i = 0; i < bytes.length; i++) {
    out += String.fromCharCode(bytes[i] ?? 0)
  }
  return out
}

export function bytesToBinaryString(bytes: Uint8Array): string {
  let binary = ""
  for (let i = 0; i < bytes.length; i += BINARY_CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(i, i + BINARY_CHUNK))
  }
  return binary
}

export function binaryStringToBytes(binary: string): Uint8Array {
  const bytes = new Uint8Array(binary.length)
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i)
  }
  return bytes
}
