import "./webcrypto-test"
import { describe, expect, it } from "vitest"
import { CryptoToolError } from "./encoding"
import { runSymmetric } from "./symmetric"

const aesCbc = {
  algorithm: "aes-128" as const,
  mode: "cbc" as const,
  padding: "pkcs7" as const,
  key: "1234567890123456",
  keyEncoding: "utf8" as const,
  iv: "1234567890123456",
  ivEncoding: "utf8" as const,
  inputEncoding: "utf8" as const,
  outputEncoding: "base64" as const,
  prependIv: false,
}

describe("symmetric cipher", () => {
  it("round-trips AES-CBC PKCS7", async () => {
    const ciphertext = await runSymmetric({
      ...aesCbc,
      direction: "encrypt",
      input: "Hello, Codeg",
    })
    const plain = await runSymmetric({
      ...aesCbc,
      direction: "decrypt",
      input: ciphertext,
      inputEncoding: "base64",
      outputEncoding: "utf8",
    })
    expect(plain).toBe("Hello, Codeg")
  })

  it("round-trips SM4 CBC PKCS7", async () => {
    const params = {
      algorithm: "sm4" as const,
      mode: "cbc" as const,
      padding: "pkcs7" as const,
      key: "0123456789abcdeffedcba9876543210",
      keyEncoding: "hex" as const,
      iv: "fedcba98765432100123456789abcdef",
      ivEncoding: "hex" as const,
      inputEncoding: "utf8" as const,
      outputEncoding: "hex" as const,
      prependIv: false,
    }
    const ciphertext = await runSymmetric({
      ...params,
      direction: "encrypt",
      input: "sm4 round-trip",
    })
    const plain = await runSymmetric({
      ...params,
      direction: "decrypt",
      input: ciphertext,
      inputEncoding: "hex",
      outputEncoding: "utf8",
    })
    expect(plain).toBe("sm4 round-trip")
  })

  it("matches SM4 ECB one-block test vector", async () => {
    const ciphertext = await runSymmetric({
      algorithm: "sm4",
      mode: "ecb",
      padding: "none",
      direction: "encrypt",
      key: "0123456789abcdeffedcba9876543210",
      keyEncoding: "hex",
      iv: "",
      ivEncoding: "hex",
      input: "0123456789abcdeffedcba9876543210",
      inputEncoding: "hex",
      outputEncoding: "hex",
      prependIv: false,
    })
    expect(ciphertext).toBe("681edf34d206965e86b3e94f536e4246")
  })

  it("rejects a wrong-length key with the expected byte count", async () => {
    await expect(
      runSymmetric({
        ...aesCbc,
        direction: "encrypt",
        key: "short",
        input: "hi",
      })
    ).rejects.toMatchObject({
      code: "key-length",
      message: expect.stringMatching(/16 bytes/),
    })
  })

  it("rejects a wrong-length IV with the expected byte count", async () => {
    await expect(
      runSymmetric({
        ...aesCbc,
        direction: "encrypt",
        iv: "short",
        input: "hi",
      })
    ).rejects.toMatchObject({
      code: "iv-length",
      message: expect.stringMatching(/16 bytes/),
    })
  })

  it("rejects illegal ciphertext encoding", async () => {
    await expect(
      runSymmetric({
        ...aesCbc,
        direction: "decrypt",
        input: "%%%%",
        inputEncoding: "base64",
        outputEncoding: "utf8",
      })
    ).rejects.toBeInstanceOf(CryptoToolError)
  })

  it("reports GCM authentication failure", async () => {
    const ciphertext = await runSymmetric({
      ...aesCbc,
      mode: "gcm",
      iv: "00112233445566778899aabb",
      ivEncoding: "hex",
      direction: "encrypt",
      input: "auth me",
      outputEncoding: "hex",
    })
    const tampered =
      ciphertext.slice(0, -2) + (ciphertext.endsWith("00") ? "01" : "00")
    await expect(
      runSymmetric({
        ...aesCbc,
        mode: "gcm",
        iv: "00112233445566778899aabb",
        ivEncoding: "hex",
        direction: "decrypt",
        input: tampered,
        inputEncoding: "hex",
        outputEncoding: "utf8",
      })
    ).rejects.toMatchObject({
      code: "gcm-auth",
      message: expect.stringMatching(/authentication failed/i),
    })
  })

  it("reports bad PKCS7 padding for a truncated ciphertext", async () => {
    await expect(
      runSymmetric({
        ...aesCbc,
        padding: "pkcs7",
        direction: "decrypt",
        input: "00112233445566778899aabbccddee",
        inputEncoding: "hex",
        outputEncoding: "utf8",
      })
    ).rejects.toMatchObject({ code: "bad-padding" })
  })
})
