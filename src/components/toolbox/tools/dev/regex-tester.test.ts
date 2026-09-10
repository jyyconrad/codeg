import { describe, expect, it } from "vitest"
import {
  MAX_REGEX_INPUT,
  MAX_REGEX_PATTERN,
  testRegex,
} from "./regex-tester.core"

describe("testRegex", () => {
  it("reports match groups and a replace preview", () => {
    const result = testRegex("a@b.com and c@d.org", {
      pattern: "(\\w+)@(\\w+\\.\\w+)",
      flags: "g",
      replacement: "$1 at $2",
    })
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.matches).toHaveLength(2)
    expect(result.matches[0].groups).toEqual(["a", "b.com"])
    expect(result.replaced).toBe("a at b.com and c at d.org")
    expect(result.output).toContain('$1 = "a"')
    expect(result.output).toContain("Replace preview:")
  })

  it("captures named groups", () => {
    const result = testRegex("id=42", {
      pattern: "id=(?<n>\\d+)",
      flags: "",
      replacement: "#$<n>",
    })
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.matches[0].named).toEqual({ n: "42" })
    expect(result.replaced).toBe("#42")
  })

  it("returns a syntax error for bad patterns", () => {
    const result = testRegex("abc", {
      pattern: "(",
      flags: "g",
      replacement: "",
    })
    expect(result.ok).toBe(false)
  })

  it("caps catastrophic-sized input and patterns", () => {
    const longInput = testRegex("a".repeat(MAX_REGEX_INPUT + 1), {
      pattern: "a+",
      flags: "g",
      replacement: "",
    })
    const longPattern = testRegex("a", {
      pattern: "a".repeat(MAX_REGEX_PATTERN + 1),
      flags: "g",
      replacement: "",
    })
    expect(longInput.ok).toBe(false)
    expect(longPattern.ok).toBe(false)
    if (longInput.ok || longPattern.ok) return
    expect(longInput.error).toMatch(/Input exceeds/)
    expect(longPattern.error).toMatch(/Pattern exceeds/)
  })

  it("does not infinite-loop on empty matches", () => {
    const result = testRegex("abc", {
      pattern: "",
      flags: "g",
      replacement: "-",
    })
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.output).toBe("")
  })
})
