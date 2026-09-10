import { describe, expect, it } from "vitest"
import { decodeJwt, formatJwtDecode } from "./jwt"

const TOKEN =
  "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c"

describe("jwt decode", () => {
  it("splits header and payload without verifying", () => {
    const decoded = decodeJwt(TOKEN)
    expect(decoded.alg).toBe("HS256")
    expect(decoded.header).toEqual({ alg: "HS256", typ: "JWT" })
    expect(decoded.payload).toEqual({
      sub: "1234567890",
      name: "John Doe",
      iat: 1516239022,
    })
    const text = formatJwtDecode(decoded)
    expect(text).toContain("Decode does not mean the token is valid.")
    expect(text).toContain('"alg": "HS256"')
  })

  it("rejects a token that is not three parts", () => {
    expect(() => decodeJwt("only.two")).toThrow(/three Base64URL parts/)
  })
})
