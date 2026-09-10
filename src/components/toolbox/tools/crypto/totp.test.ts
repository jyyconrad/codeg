import "./webcrypto-test"
import { describe, expect, it } from "vitest"
import { encodeUtf8 } from "./encoding"
import { decodeBase32, generateTotp, type TotpHmacFn } from "./totp"

describe("totp", () => {
  it("computes a code from a stub HMAC (RFC 4226 truncation)", async () => {
    const hmac: TotpHmacFn = () =>
      Uint8Array.from([
        0x1f, 0x86, 0x98, 0x69, 0x0e, 0x02, 0xca, 0x16, 0x61, 0x85, 0x50, 0xef,
        0x7f, 0x19, 0xda, 0x8e, 0x94, 0x5b, 0x55, 0x5a,
      ])
    const code = await generateTotp({
      secret: new Uint8Array(20),
      unixSeconds: 0,
      digits: 6,
      hmac,
    })
    expect(code).toBe("872921")
  })

  it("matches RFC 6238 SHA-1 at t=59", async () => {
    const code = await generateTotp({
      secret: encodeUtf8("12345678901234567890"),
      unixSeconds: 59,
      digits: 8,
      algorithm: "SHA-1",
    })
    expect(code).toBe("94287082")
  })

  it("decodes Base32 secrets", () => {
    expect(Array.from(decodeBase32("MFRGG"))).toEqual([0x61, 0x62, 0x63])
  })
})
