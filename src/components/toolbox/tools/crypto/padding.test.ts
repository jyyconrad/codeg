import { describe, expect, it } from "vitest"
import { CryptoToolError } from "./encoding"
import { padBytes, unpadBytes } from "./padding"

describe("padding", () => {
  it("applies and strips PKCS7", () => {
    const padded = padBytes(Uint8Array.from([1, 2, 3]), "pkcs7")
    expect(padded.length).toBe(16)
    expect(padded[15]).toBe(13)
    expect(Array.from(unpadBytes(padded, "pkcs7"))).toEqual([1, 2, 3])
  })

  it("rejects invalid PKCS7", () => {
    expect(() => unpadBytes(new Uint8Array(16), "pkcs7")).toThrow(
      CryptoToolError
    )
  })
})
