import "./webcrypto-test"
import { describe, expect, it } from "vitest"
import { encodeHex, encodeUtf8 } from "./encoding"
import { hmacBytes } from "./hmac.core"

describe("hmac", () => {
  it("matches the HMAC-SHA256 RFC 4231 case 1 vector", async () => {
    const key = new Uint8Array(20).fill(0x0b)
    const mac = await hmacBytes("SHA-256", key, encodeUtf8("Hi There"))
    expect(encodeHex(mac)).toBe(
      "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
    )
  })

  it("does not truncate a UTF-8 key", async () => {
    const key = encodeUtf8("a".repeat(40))
    expect(key.length).toBe(40)
    const mac = await hmacBytes("SHA-256", key, encodeUtf8("msg"))
    expect(mac.length).toBe(32)
  })
})
