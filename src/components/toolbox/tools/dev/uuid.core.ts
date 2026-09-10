export type UuidVersion = "v4" | "v7"

export type UuidResult =
  | { ok: true; output: string; ids: string[] }
  | { ok: false; error: string }

export const UUID_MIN_COUNT = 1
export const UUID_MAX_COUNT = 100

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/

function getCrypto(): Crypto {
  const cryptoObj = globalThis.crypto
  if (!cryptoObj || typeof cryptoObj.getRandomValues !== "function") {
    throw new Error("Secure random is unavailable")
  }
  return cryptoObj
}

function bytesToUuid(bytes: Uint8Array): string {
  const hex = Array.from(bytes, (byte) =>
    byte.toString(16).padStart(2, "0")
  ).join("")
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`
}

export function generateUuidV4(): string {
  const cryptoObj = getCrypto()
  if (typeof cryptoObj.randomUUID === "function") {
    return cryptoObj.randomUUID()
  }
  const bytes = new Uint8Array(16)
  cryptoObj.getRandomValues(bytes)
  bytes[6] = (bytes[6] & 0x0f) | 0x40
  bytes[8] = (bytes[8] & 0x3f) | 0x80
  return bytesToUuid(bytes)
}

export function generateUuidV7(
  nowMs: number = Date.now(),
  randomBytes?: Uint8Array
): string {
  const cryptoObj = getCrypto()
  const rand = randomBytes ?? cryptoObj.getRandomValues(new Uint8Array(10))
  if (rand.length < 10) {
    throw new Error("UUID v7 needs 10 random bytes")
  }
  const ts = BigInt(Math.max(0, Math.trunc(nowMs))) & 0xffffffffffffn
  const bytes = new Uint8Array(16)
  bytes[0] = Number((ts >> 40n) & 0xffn)
  bytes[1] = Number((ts >> 32n) & 0xffn)
  bytes[2] = Number((ts >> 24n) & 0xffn)
  bytes[3] = Number((ts >> 16n) & 0xffn)
  bytes[4] = Number((ts >> 8n) & 0xffn)
  bytes[5] = Number(ts & 0xffn)
  bytes[6] = (rand[0] & 0x0f) | 0x70
  bytes[7] = rand[1]
  bytes[8] = (rand[2] & 0x3f) | 0x80
  bytes[9] = rand[3]
  bytes[10] = rand[4]
  bytes[11] = rand[5]
  bytes[12] = rand[6]
  bytes[13] = rand[7]
  bytes[14] = rand[8]
  bytes[15] = rand[9]
  return bytesToUuid(bytes)
}

export function isUuid(value: string): boolean {
  return UUID_RE.test(value)
}

export function generateUuidBatch(
  version: UuidVersion,
  count: number
): UuidResult {
  const n = Math.trunc(count)
  if (!Number.isFinite(n) || n < UUID_MIN_COUNT || n > UUID_MAX_COUNT) {
    return {
      ok: false,
      error: `Count must be between ${UUID_MIN_COUNT} and ${UUID_MAX_COUNT}`,
    }
  }
  try {
    const ids: string[] = []
    for (let i = 0; i < n; i++) {
      ids.push(version === "v7" ? generateUuidV7() : generateUuidV4())
    }
    return { ok: true, output: ids.join("\n"), ids }
  } catch (error) {
    return {
      ok: false,
      error: error instanceof Error ? error.message : String(error),
    }
  }
}
