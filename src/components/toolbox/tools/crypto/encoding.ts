export type ByteEncoding = "utf8" | "hex" | "base64"

export type CryptoErrorCode =
  | "key-length"
  | "iv-length"
  | "illegal-encoding"
  | "gcm-auth"
  | "bad-padding"
  | "invalid-input"
  | "unsupported"

export class CryptoToolError extends Error {
  readonly code: CryptoErrorCode

  constructor(code: CryptoErrorCode, message: string) {
    super(message)
    this.name = "CryptoToolError"
    this.code = code
  }
}

export function errorMessage(err: unknown): string {
  if (err instanceof CryptoToolError) return err.message
  if (err instanceof Error && err.message) return err.message
  return String(err)
}

export function getSubtle(): SubtleCrypto {
  const subtle = globalThis.crypto?.subtle
  if (!subtle) {
    throw new CryptoToolError(
      "unsupported",
      "Web Crypto is not available in this environment."
    )
  }
  return subtle
}

export function randomBytes(length: number): Uint8Array {
  const out = new Uint8Array(length)
  crypto.getRandomValues(out)
  return out
}

/** Copy into this realm's Uint8Array so `instanceof` checks in CJS libs pass under jsdom. */
export function cloneBytes(data: Uint8Array): Uint8Array {
  const copy = new Uint8Array(data.length)
  copy.set(data)
  return copy
}

export function concatBytes(...parts: Uint8Array[]): Uint8Array {
  const total = parts.reduce((sum, part) => sum + part.length, 0)
  const out = new Uint8Array(total)
  let offset = 0
  for (const part of parts) {
    out.set(part, offset)
    offset += part.length
  }
  return out
}

export function bytesEqual(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false
  let diff = 0
  for (let i = 0; i < a.length; i += 1) diff |= a[i]! ^ b[i]!
  return diff === 0
}

export function encodeUtf8(text: string): Uint8Array {
  return new TextEncoder().encode(text)
}

export function decodeUtf8(bytes: Uint8Array): string {
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes)
  } catch {
    throw new CryptoToolError("illegal-encoding", "Bytes are not valid UTF-8.")
  }
}

export function encodeHex(bytes: Uint8Array): string {
  let out = ""
  for (const byte of bytes) out += byte.toString(16).padStart(2, "0")
  return out
}

export function decodeHex(input: string): Uint8Array {
  let hex = input.replace(/\s+/g, "").replace(/:/g, "")
  if (hex.startsWith("0x") || hex.startsWith("0X")) hex = hex.slice(2)
  if (hex.length === 0) return new Uint8Array()
  if (hex.length % 2 !== 0 || /[^0-9a-fA-F]/.test(hex)) {
    throw new CryptoToolError("illegal-encoding", "Value is not valid hex.")
  }
  const out = new Uint8Array(hex.length / 2)
  for (let i = 0; i < out.length; i += 1) {
    out[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16)
  }
  return out
}

export function encodeBase64(bytes: Uint8Array): string {
  let bin = ""
  for (const byte of bytes) bin += String.fromCharCode(byte)
  return btoa(bin)
}

export function decodeBase64(input: string): Uint8Array {
  const compact = input.replace(/\s+/g, "")
  if (compact.length === 0) return new Uint8Array()
  const padded = compact + "=".repeat((4 - (compact.length % 4)) % 4)
  if (!/^[A-Za-z0-9+/]+={0,2}$/.test(padded) || padded.length % 4 !== 0) {
    throw new CryptoToolError("illegal-encoding", "Value is not valid Base64.")
  }
  try {
    const bin = atob(padded)
    const out = new Uint8Array(bin.length)
    for (let i = 0; i < bin.length; i += 1) out[i] = bin.charCodeAt(i)
    return out
  } catch {
    throw new CryptoToolError("illegal-encoding", "Value is not valid Base64.")
  }
}

export function encodeBase64Url(bytes: Uint8Array): string {
  return encodeBase64(bytes)
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/g, "")
}

export function decodeBase64Url(input: string): Uint8Array {
  const compact = input.replace(/\s+/g, "")
  if (compact.length === 0) return new Uint8Array()
  if (!/^[A-Za-z0-9_-]+$/.test(compact.replace(/=+$/g, ""))) {
    throw new CryptoToolError(
      "illegal-encoding",
      "Value is not valid Base64URL."
    )
  }
  return decodeBase64(compact.replace(/-/g, "+").replace(/_/g, "/"))
}

export function decodeBytes(encoding: ByteEncoding, input: string): Uint8Array {
  if (encoding === "utf8") return encodeUtf8(input)
  if (encoding === "hex") return decodeHex(input)
  return decodeBase64(input)
}

export function encodeBytes(encoding: ByteEncoding, bytes: Uint8Array): string {
  if (encoding === "utf8") return decodeUtf8(bytes)
  if (encoding === "hex") return encodeHex(bytes)
  return encodeBase64(bytes)
}

export function expectLength(
  bytes: Uint8Array,
  expected: number,
  kind: "key-length" | "iv-length",
  label: string
): void {
  if (bytes.length === expected) return
  throw new CryptoToolError(
    kind,
    `${label} must be ${expected} bytes (got ${bytes.length}).`
  )
}

export function expectOneOfLengths(
  bytes: Uint8Array,
  expected: readonly number[],
  kind: "key-length" | "iv-length",
  label: string
): void {
  if (expected.includes(bytes.length)) return
  throw new CryptoToolError(
    kind,
    `${label} must be ${expected.join(" or ")} bytes (got ${bytes.length}).`
  )
}
