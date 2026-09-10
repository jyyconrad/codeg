import CryptoJS from "crypto-js"
import { sm3 } from "sm-crypto-v2"
import { CryptoToolError, cloneBytes, encodeHex, getSubtle } from "./encoding"
import { sha3_256 } from "./sha3"
import { bytesToWordArray, wordArrayToBytes } from "./word-array"

export const HASH_ALGORITHMS = [
  "MD5",
  "SHA-1",
  "SHA-256",
  "SHA-384",
  "SHA-512",
  "SHA3-256",
  "SM3",
] as const

export type HashAlgorithm = (typeof HASH_ALGORITHMS)[number]

export const WEAK_HASH_ALGORITHMS: ReadonlySet<HashAlgorithm> = new Set([
  "MD5",
  "SHA-1",
])

export const SMALL_FILE_MAX_BYTES = 8 * 1024 * 1024

export type HashReport = Record<HashAlgorithm, string>

async function digestWeb(name: string, data: Uint8Array): Promise<Uint8Array> {
  const out = await getSubtle().digest(name, data)
  return new Uint8Array(out)
}

function md5(data: Uint8Array): Uint8Array {
  return wordArrayToBytes(CryptoJS.MD5(bytesToWordArray(data)))
}

export async function hashBytes(data: Uint8Array): Promise<HashReport> {
  const [sha1, sha256, sha384, sha512] = await Promise.all([
    digestWeb("SHA-1", data),
    digestWeb("SHA-256", data),
    digestWeb("SHA-384", data),
    digestWeb("SHA-512", data),
  ])
  return {
    MD5: encodeHex(md5(data)),
    "SHA-1": encodeHex(sha1),
    "SHA-256": encodeHex(sha256),
    "SHA-384": encodeHex(sha384),
    "SHA-512": encodeHex(sha512),
    "SHA3-256": encodeHex(sha3_256(data)),
    SM3: sm3(cloneBytes(data)),
  }
}

export async function readFileBytes(file: File): Promise<Uint8Array> {
  if (file.size > SMALL_FILE_MAX_BYTES) {
    throw new CryptoToolError(
      "invalid-input",
      `File is larger than ${SMALL_FILE_MAX_BYTES} bytes. Large-file hashing is not in v1.`
    )
  }
  if (typeof file.stream === "function") {
    const reader = file.stream().getReader()
    const chunks: Uint8Array[] = []
    let total = 0
    for (;;) {
      const { done, value } = await reader.read()
      if (done) break
      chunks.push(value)
      total += value.byteLength
      if (total > SMALL_FILE_MAX_BYTES) {
        throw new CryptoToolError(
          "invalid-input",
          `File is larger than ${SMALL_FILE_MAX_BYTES} bytes. Large-file hashing is not in v1.`
        )
      }
    }
    const out = new Uint8Array(total)
    let offset = 0
    for (const chunk of chunks) {
      out.set(chunk, offset)
      offset += chunk.byteLength
    }
    return out
  }
  return new Uint8Array(await file.arrayBuffer())
}

export function formatHashReport(report: HashReport): string {
  return HASH_ALGORITHMS.map((name) => {
    const note = WEAK_HASH_ALGORITHMS.has(name)
      ? "  (checksum only, not for passwords)"
      : ""
    return `${name}${note}\n${report[name]}`
  }).join("\n\n")
}
