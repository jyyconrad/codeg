import { CryptoToolError } from "./encoding"

export const BLOCK_SIZE = 16

export type PaddingMode = "pkcs7" | "zero" | "none"

export function padBytes(
  data: Uint8Array,
  padding: PaddingMode,
  blockSize = BLOCK_SIZE
): Uint8Array {
  if (padding === "none") {
    if (data.length % blockSize !== 0) {
      throw new CryptoToolError(
        "bad-padding",
        `NoPadding requires input length to be a multiple of ${blockSize} bytes (got ${data.length}).`
      )
    }
    return data
  }
  if (padding === "zero") {
    const rem = data.length % blockSize
    if (rem === 0) return data
    const out = new Uint8Array(data.length + (blockSize - rem))
    out.set(data)
    return out
  }
  const n = blockSize - (data.length % blockSize)
  const out = new Uint8Array(data.length + n)
  out.set(data)
  out.fill(n, data.length)
  return out
}

export function unpadBytes(
  data: Uint8Array,
  padding: PaddingMode,
  blockSize = BLOCK_SIZE
): Uint8Array {
  if (padding === "none") {
    if (data.length % blockSize !== 0) {
      throw new CryptoToolError(
        "bad-padding",
        `NoPadding requires ciphertext length to be a multiple of ${blockSize} bytes (got ${data.length}).`
      )
    }
    return data
  }
  if (padding === "zero") {
    let end = data.length
    while (end > 0 && data[end - 1] === 0) end -= 1
    return data.subarray(0, end)
  }
  if (data.length === 0 || data.length % blockSize !== 0) {
    throw new CryptoToolError(
      "bad-padding",
      "PKCS7 padding is invalid: ciphertext length is not a multiple of the block size."
    )
  }
  const n = data[data.length - 1]!
  if (n < 1 || n > blockSize) {
    throw new CryptoToolError("bad-padding", "PKCS7 padding is invalid.")
  }
  for (let i = 1; i <= n; i += 1) {
    if (data[data.length - i] !== n) {
      throw new CryptoToolError("bad-padding", "PKCS7 padding is invalid.")
    }
  }
  return data.subarray(0, data.length - n)
}
