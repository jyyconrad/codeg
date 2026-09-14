import { describe, expect, it } from "vitest"
import { EncodingError } from "./error"
import {
  decodeJsonEscapes,
  decodeUnicode,
  decodeUnicodeEscapes,
  encodeJsonEscapes,
  encodeUnicode,
  encodeUnicodeEscapes,
} from "./unicode-escape"

describe("unicode-escape", () => {
  it("encodes non-ASCII as \\uXXXX", () => {
    expect(encodeUnicodeEscapes("你好")).toBe("\\u4f60\\u597d")
    expect(encodeUnicodeEscapes("Hi你")).toBe("Hi\\u4f60")
    expect(encodeUnicodeEscapes("😀")).toBe("\\ud83d\\ude00")
  })

  it("round-trips unicode escapes", () => {
    const samples = ["", "ASCII", "你好", "emoji 😀", "mix Hi世界"]
    for (const sample of samples) {
      expect(decodeUnicodeEscapes(encodeUnicodeEscapes(sample))).toBe(sample)
      expect(decodeUnicode(encodeUnicode(sample, "unicode"), "unicode")).toBe(
        sample
      )
    }
  })

  it("round-trips JSON-style escapes", () => {
    const samples = ["", 'Hello\n"世界"', "😀", "tab\tand\\slash"]
    for (const sample of samples) {
      expect(decodeJsonEscapes(encodeJsonEscapes(sample))).toBe(sample)
      expect(decodeUnicode(encodeUnicode(sample, "json"), "json")).toBe(sample)
    }
    expect(encodeJsonEscapes("你好")).toBe("\\u4f60\\u597d")
    expect(encodeJsonEscapes('a"b')).toBe('a\\"b')
    expect(decodeJsonEscapes('"hello\\nworld"')).toBe("hello\nworld")
  })

  it("rejects incomplete or illegal escapes", () => {
    expect(() => decodeUnicodeEscapes("\\u12")).toThrow(EncodingError)
    expect(() => decodeUnicodeEscapes("\\uXXXX")).toThrow(EncodingError)
    expect(() => decodeUnicodeEscapes("\\n")).toThrow(EncodingError)
    expect(() => decodeJsonEscapes("\\q")).toThrow(EncodingError)
    expect(() => decodeJsonEscapes('"unterminated')).toThrow(EncodingError)
  })
})
