import { CryptoToolError, getSubtle } from "./encoding"

export type TotpAlgorithm = "SHA-1" | "SHA-256" | "SHA-512"

const BASE32 = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"

export function decodeBase32(input: string): Uint8Array {
  const compact = input.replace(/\s+/g, "").replace(/=+$/g, "").toUpperCase()
  if (compact.length === 0) return new Uint8Array()
  if (/[^A-Z2-7]/.test(compact)) {
    throw new CryptoToolError(
      "illegal-encoding",
      "Secret is not valid Base32 (A-Z, 2-7)."
    )
  }
  let bits = 0
  let buffer = 0
  const out: number[] = []
  for (const ch of compact) {
    const value = BASE32.indexOf(ch)
    buffer = (buffer << 5) | value
    bits += 5
    if (bits >= 8) {
      bits -= 8
      out.push((buffer >> bits) & 0xff)
    }
  }
  return new Uint8Array(out)
}

export function counterToBytes(counter: number): Uint8Array {
  const out = new Uint8Array(8)
  let n = BigInt(counter)
  for (let i = 7; i >= 0; i -= 1) {
    out[i] = Number(n & 0xffn)
    n >>= 8n
  }
  return out
}

export function dynamicTruncate(hmac: Uint8Array, digits: number): string {
  const offset = hmac[hmac.length - 1]! & 0x0f
  const bin =
    ((hmac[offset]! & 0x7f) << 24) |
    ((hmac[offset + 1]! & 0xff) << 16) |
    ((hmac[offset + 2]! & 0xff) << 8) |
    (hmac[offset + 3]! & 0xff)
  const mod = 10 ** digits
  return String(bin % mod).padStart(digits, "0")
}

export type TotpHmacFn = (
  algorithm: TotpAlgorithm,
  key: Uint8Array,
  message: Uint8Array
) => Promise<Uint8Array> | Uint8Array

export async function webCryptoHmac(
  algorithm: TotpAlgorithm,
  key: Uint8Array,
  message: Uint8Array
): Promise<Uint8Array> {
  const cryptoKey = await getSubtle().importKey(
    "raw",
    key,
    { name: "HMAC", hash: algorithm },
    false,
    ["sign"]
  )
  return new Uint8Array(await getSubtle().sign("HMAC", cryptoKey, message))
}

export async function generateTotp(params: {
  secret: Uint8Array
  unixSeconds: number
  period?: number
  digits?: number
  algorithm?: TotpAlgorithm
  hmac?: TotpHmacFn
}): Promise<string> {
  const period = params.period ?? 30
  const digits = params.digits ?? 6
  const algorithm = params.algorithm ?? "SHA-1"
  if (period < 1 || digits < 1) {
    throw new CryptoToolError("invalid-input", "Invalid TOTP period or digits.")
  }
  const counter = Math.floor(params.unixSeconds / period)
  const hmacFn = params.hmac ?? webCryptoHmac
  const mac = await hmacFn(algorithm, params.secret, counterToBytes(counter))
  return dynamicTruncate(mac, digits)
}

export function totpRemaining(unixSeconds: number, period = 30): number {
  return period - (unixSeconds % period)
}

export function parseTotpSecret(input: string): Uint8Array {
  const trimmed = input.trim()
  const otpauth = trimmed.match(/secret=([A-Z2-7=]+)/i)
  const secret = otpauth ? otpauth[1]! : trimmed
  const bytes = decodeBase32(secret)
  if (bytes.length === 0) {
    throw new CryptoToolError("invalid-input", "TOTP secret is empty.")
  }
  return bytes
}
