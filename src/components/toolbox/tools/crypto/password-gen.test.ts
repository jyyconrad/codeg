import { describe, expect, it } from "vitest"
import { charsetFromOptions, generatePassword } from "./password-gen"
import { randomInt } from "./random-int"

describe("password generator", () => {
  it("only emits characters from the chosen charset", () => {
    const options = {
      length: 64,
      sets: ["digits" as const],
    }
    const charset = charsetFromOptions(options)
    expect(charset).toBe("0123456789")
    const password = generatePassword(options)
    expect(password).toHaveLength(64)
    expect([...password].every((ch) => charset.includes(ch))).toBe(true)
  })

  it("rejects an empty charset", () => {
    expect(() => generatePassword({ length: 8, sets: [] })).toThrow(
      /character set/
    )
  })

  it("rejects modulo-biased samples", () => {
    const values = [255, 251, 0]
    const fill = (buffer: Uint8Array) => {
      buffer[0] = values.shift() ?? 0
      return buffer
    }
    expect(randomInt(10, fill)).toBe(0)
    expect(values).toEqual([])
  })
})
