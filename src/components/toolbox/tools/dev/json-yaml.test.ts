import { describe, expect, it } from "vitest"
import { convertJsonYaml } from "./json-yaml.core"

describe("convertJsonYaml", () => {
  it("converts JSON objects to YAML", () => {
    const result = convertJsonYaml(
      '{"name":"Ada","ok":true,"count":2}',
      "json-to-yaml"
    )
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.output).toContain("name: Ada")
    expect(result.output).toContain("ok: true")
    expect(result.output).toContain("count: 2")
  })

  it("converts YAML back to JSON and keeps basic types", () => {
    const result = convertJsonYaml(
      "name: Ada\nok: true\nitems:\n  - 1\n  - two\n",
      "yaml-to-json"
    )
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(JSON.parse(result.output)).toEqual({
      name: "Ada",
      ok: true,
      items: [1, "two"],
    })
  })

  it("reports JSON parse failures with line and column", () => {
    const result = convertJsonYaml('{\n  "a": tru}', "json-to-yaml")
    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toMatch(/at line 2, column 8/)
  })

  it("reports YAML failures with line and column", () => {
    const result = convertJsonYaml("a: [", "yaml-to-json")
    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toMatch(/at line 2, column 1/)
  })

  it("does not coerce YAML 1.1 yes/on with the JSON schema", () => {
    const result = convertJsonYaml("k: yes\n", "yaml-to-json")
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(JSON.parse(result.output)).toEqual({ k: "yes" })
  })
})
