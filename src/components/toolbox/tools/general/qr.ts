export const QR_ECC_LEVELS = ["L", "M", "Q", "H"] as const
export type QrEccLevel = (typeof QR_ECC_LEVELS)[number]

export function normalizeQrPayload(raw: string): string {
  return raw.replace(/^\uFEFF/, "")
}

export function isQrEccLevel(value: string): value is QrEccLevel {
  return (QR_ECC_LEVELS as readonly string[]).includes(value)
}

export function qrLooksLikeUrl(payload: string): boolean {
  const trimmed = payload.trim()
  if (!trimmed) return false
  try {
    const url = new URL(trimmed)
    return url.protocol === "http:" || url.protocol === "https:"
  } catch {
    return false
  }
}

/** Decode result as plain text only — never a navigable URL. */
export function formatQrDecodeResult(payload: string): string {
  return payload
}

export function shouldAutoOpenQrPayload(payload: string): false {
  void payload
  return false
}
