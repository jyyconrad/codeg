import { describe, expect, it } from "vitest"
import { generateSm2KeyPair, sm2Decrypt, sm2Encrypt } from "./sm2"

describe("sm2", () => {
  it("round-trips encrypt/decrypt with C1C3C2", async () => {
    const pair = await generateSm2KeyPair(true)
    const ciphertext = await sm2Encrypt({
      publicKey: pair.publicKey,
      plaintext: "hello sm2",
      inputEncoding: "utf8",
      outputEncoding: "hex",
      cipherMode: "c1c3c2",
      asn1: false,
    })
    const plain = await sm2Decrypt({
      privateKey: pair.privateKey,
      ciphertext,
      inputEncoding: "hex",
      outputEncoding: "utf8",
      cipherMode: "c1c3c2",
      asn1: false,
    })
    expect(plain).toBe("hello sm2")
  })
})
