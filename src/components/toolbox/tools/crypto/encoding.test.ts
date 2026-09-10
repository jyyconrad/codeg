import { describe, expect, it } from "vitest"
import {
  CryptoToolError,
  decodeBase64,
  decodeHex,
  encodeHex,
  encodeUtf8,
} from "./encoding"

describe("encoding", () => {
  it("decodes hex with spaces and 0x prefix", () => {
    expect(Array.from(decodeHex("0x 61 62 63"))).toEqual([0x61, 0x62, 0x63])
  })

  it("rejects illegal hex", () => {
    expect(() => decodeHex("zz")).toThrow(CryptoToolError)
    expect(() => decodeHex("abc")).toThrow(/not valid hex/)
  })

  it("rejects illegal Base64", () => {
    expect(() => decodeBase64("@@@@")).toThrow(/not valid Base64/)
  })

  it("round-trips utf8 bytes", () => {
    expect(encodeHex(encodeUtf8("abc"))).toBe("616263")
  })
})
