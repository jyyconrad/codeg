import { describe, expect, it } from "vitest"
import {
  formatQrDecodeResult,
  isQrEccLevel,
  normalizeQrPayload,
  qrLooksLikeUrl,
  shouldAutoOpenQrPayload,
} from "./qr"

describe("qr payload handling", () => {
  it("normalizes text without opening links", () => {
    expect(normalizeQrPayload("\uFEFFhello")).toBe("hello")
    expect(qrLooksLikeUrl("https://example.com/path")).toBe(true)
    expect(qrLooksLikeUrl("javascript:alert(1)")).toBe(false)
    expect(qrLooksLikeUrl("not a url")).toBe(false)
    expect(shouldAutoOpenQrPayload("https://example.com")).toBe(false)
    expect(formatQrDecodeResult("https://example.com")).toBe(
      "https://example.com"
    )
    expect(isQrEccLevel("H")).toBe(true)
    expect(isQrEccLevel("Z")).toBe(false)
  })
})
