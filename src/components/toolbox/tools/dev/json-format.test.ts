import { describe, expect, it } from "vitest"
import { formatJson, integerLosesPrecision } from "./json-format.core"

describe("formatJson", () => {
  it("pretty-prints objects with 2-space indent", () => {
    const result = formatJson('{"b":2,"a":1}', {
      mode: "pretty",
      stringifyBigIntegers: false,
    })
    expect(result).toEqual({
      ok: true,
      output: `{
  "b": 2,
  "a": 1
}`,
    })
  })

  it("minifies JSON", () => {
    const result = formatJson('{\n  "hello": "world"\n}', {
      mode: "minify",
      stringifyBigIntegers: false,
    })
    expect(result).toEqual({ ok: true, output: '{"hello":"world"}' })
  })

  it("reports parse errors with line and column", () => {
    const result = formatJson('{\n  "a": tru}', {
      mode: "pretty",
      stringifyBigIntegers: false,
    })
    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toMatch(/at line 2, column 8/)
  })

  it("warns when JSON.parse would lose integer precision", () => {
    expect(integerLosesPrecision("9007199254740993")).toBe(true)
    const result = formatJson("9007199254740993", {
      mode: "minify",
      stringifyBigIntegers: false,
    })
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.output).toBe("9007199254740992")
    expect(result.warning).toMatch(/9007199254740993 lost precision/)
  })

  it("stringifies unsafe integers instead of rounding them", () => {
    const result = formatJson('{"id":9007199254740993}', {
      mode: "minify",
      stringifyBigIntegers: true,
    })
    expect(result).toEqual({
      ok: true,
      output: '{"id":"9007199254740993"}',
    })
  })

  it("does not treat safe integers as precision loss", () => {
    const result = formatJson("[42,9007199254740991]", {
      mode: "minify",
      stringifyBigIntegers: false,
    })
    expect(result).toEqual({ ok: true, output: "[42,9007199254740991]" })
  })

  it("ignores digits that live inside strings", () => {
    const result = formatJson('{"id":"9007199254740993"}', {
      mode: "minify",
      stringifyBigIntegers: true,
    })
    expect(result).toEqual({
      ok: true,
      output: '{"id":"9007199254740993"}',
    })
  })

  it("returns empty output for blank input", () => {
    expect(
      formatJson("  \n", { mode: "pretty", stringifyBigIntegers: false })
    ).toEqual({ ok: true, output: "" })
  })
})
