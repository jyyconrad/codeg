import { describe, expect, it } from "vitest"
import {
  decodeBase64,
  decodeBase64Bytes,
  encodeBase64,
  encodeBase64Bytes,
} from "./base64"
import { EncodingError } from "./error"

describe("base64", () => {
  it("encodes UTF-8 text", () => {
    expect(encodeBase64("hello")).toBe("aGVsbG8=")
    expect(encodeBase64("你好")).toBe("5L2g5aW9")
  })

  it("round-trips UTF-8 text", () => {
    const samples = ["", "hello", "Hello, 世界", "emoji 😀", "line\nbreak\t"]
    for (const sample of samples) {
      expect(decodeBase64(encodeBase64(sample))).toBe(sample)
    }
  })

  it("treats whitespace-only input as empty", () => {
    expect(decodeBase64(" \n\t ")).toBe("")
  })

  it("accepts wrapped Base64 and restores padding", () => {
    expect(decodeBase64("aGVs\nbG8")).toBe("hello")
  })

  it("rejects illegal charset", () => {
    expect(() => decodeBase64("!!!!")).toThrow(EncodingError)
    expect(() => decodeBase64("aGVsbG8_")).toThrow(EncodingError)
    expect(() => decodeBase64("aGVs$G8=")).toThrow(EncodingError)
  })

  it("rejects illegal padding and length", () => {
    expect(() => decodeBase64("a")).toThrow(EncodingError)
    expect(() => decodeBase64("aGVsbG8===")).toThrow(EncodingError)
    expect(() => decodeBase64("=aGVsbG8=")).toThrow(EncodingError)
  })

  it("rejects decoded bytes that are not UTF-8", () => {
    expect(() => decodeBase64("/w==")).toThrow(EncodingError)
  })

  it("round-trips arbitrary bytes", () => {
    const bytes = Uint8Array.from([0, 1, 127, 128, 255])
    expect(decodeBase64Bytes(encodeBase64Bytes(bytes))).toEqual(bytes)
  })
})
