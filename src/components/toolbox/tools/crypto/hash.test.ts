import "./webcrypto-test"
import { describe, expect, it } from "vitest"
import { encodeUtf8 } from "./encoding"
import { hashBytes } from "./hash"

describe("hash", () => {
  it("matches the SHA-256 vector for abc", async () => {
    const report = await hashBytes(encodeUtf8("abc"))
    expect(report["SHA-256"]).toBe(
      "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    )
  })

  it("matches SHA3-256 empty and abc vectors", async () => {
    const empty = await hashBytes(new Uint8Array())
    expect(empty["SHA3-256"]).toBe(
      "a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a"
    )
    const abc = await hashBytes(encodeUtf8("abc"))
    expect(abc["SHA3-256"]).toBe(
      "3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532"
    )
  })

  it("matches SM3 abc vector", async () => {
    const report = await hashBytes(encodeUtf8("abc"))
    expect(report.SM3).toBe(
      "66c7f0f462eeedd9d1f2d46bdc10e4e24167c4875cf2f7a2297da02b8f4ba8e0"
    )
  })
})
