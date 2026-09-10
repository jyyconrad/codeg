import { describe, expect, it } from "vitest"
import { estimatePasswordStrength } from "./password-strength"

describe("password strength", () => {
  it("counts length, classes, and entropy", () => {
    const weak = estimatePasswordStrength("abc")
    expect(weak.length).toBe(3)
    expect(weak.classes).toBe(1)
    expect(weak.hasLower).toBe(true)
    expect(weak.entropyBits).toBeCloseTo(3 * Math.log2(26), 5)

    const mixed = estimatePasswordStrength("Aa1!")
    expect(mixed.classes).toBe(4)
    expect(mixed.hasDigit).toBe(true)
    expect(mixed.hasSymbol).toBe(true)
    expect(mixed.entropyBits).toBeGreaterThan(weak.entropyBits)
  })
})
