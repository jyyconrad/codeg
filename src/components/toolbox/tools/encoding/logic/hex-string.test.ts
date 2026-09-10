import { describe, expect, it } from "vitest"
import { EncodingError } from "./error"
import { decodeHex, encodeHex } from "./hex-string"

describe("hex-string", () => {
  it("encodes UTF-8 and Latin-1", () => {
    expect(encodeHex("Hi", "utf-8")).toBe("4869")
    expect(encodeHex("你好", "utf-8")).toBe("e4bda0e5a5bd")
    expect(encodeHex("ÿ", "latin1")).toBe("ff")
  })

  it("round-trips both charsets", () => {
    const utf8 = ["", "Hi", "Hello, 世界", "😀"]
    for (const sample of utf8) {
      expect(decodeHex(encodeHex(sample, "utf-8"), "utf-8")).toBe(sample)
    }
    const latin1 = ["", "Hi", "café", "ÿ"]
    for (const sample of latin1) {
      expect(decodeHex(encodeHex(sample, "latin1"), "latin1")).toBe(sample)
    }
  })

  it("tolerates spaces and 0x prefixes", () => {
    expect(decodeHex("0x48 0x69", "utf-8")).toBe("Hi")
    expect(decodeHex("0x4869", "utf-8")).toBe("Hi")
    expect(decodeHex("48\n69", "utf-8")).toBe("Hi")
    expect(decodeHex("0XFF", "latin1")).toBe("ÿ")
  })

  it("rejects illegal hex and odd length", () => {
    expect(() => decodeHex("xyz", "utf-8")).toThrow(EncodingError)
    expect(() => decodeHex("abc", "utf-8")).toThrow(EncodingError)
    expect(() => decodeHex("4g", "utf-8")).toThrow(EncodingError)
  })

  it("rejects UTF-8-invalid bytes and Latin-1-impossible text", () => {
    expect(() => decodeHex("ff", "utf-8")).toThrow(EncodingError)
    expect(() => encodeHex("你好", "latin1")).toThrow(EncodingError)
  })
})
