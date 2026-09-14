import { describe, expect, it } from "vitest"
import { EncodingError } from "./error"
import { convertRadix } from "./radix"

describe("radix", () => {
  it("converts between bases 2 and 36", () => {
    expect(convertRadix("255", 10, 16)).toBe("ff")
    expect(convertRadix("ff", 16, 10)).toBe("255")
    expect(convertRadix("11111111", 2, 10)).toBe("255")
    expect(convertRadix("z", 36, 10)).toBe("35")
    expect(convertRadix("-10", 10, 16)).toBe("-a")
    expect(convertRadix("+10", 10, 2)).toBe("1010")
    expect(convertRadix("0", 10, 2)).toBe("0")
  })

  it("round-trips big integers as strings", () => {
    const big = "123456789012345678901234567890"
    expect(convertRadix(convertRadix(big, 10, 16), 16, 10)).toBe(big)
    expect(convertRadix(convertRadix(big, 10, 36), 36, 10)).toBe(big)
    expect(Number(big).toString()).not.toBe(big)
  })

  it("rejects invalid digits and radices", () => {
    expect(() => convertRadix("2", 2, 10)).toThrow(EncodingError)
    expect(() => convertRadix("g", 16, 10)).toThrow(EncodingError)
    expect(() => convertRadix("1.5", 10, 16)).toThrow(EncodingError)
    expect(() => convertRadix("10", 1, 10)).toThrow(EncodingError)
    expect(() => convertRadix("10", 10, 37)).toThrow(EncodingError)
    expect(() => convertRadix("-", 10, 16)).toThrow(EncodingError)
  })
})
