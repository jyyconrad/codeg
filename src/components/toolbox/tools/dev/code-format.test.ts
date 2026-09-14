import { describe, expect, it } from "vitest"
import { formatCode, indentXmlFallback } from "./code-format.core"

describe("formatCode", () => {
  it("pretty-prints XML and preserves attributes", () => {
    const result = formatCode(
      '<?xml version="1.0"?><root><item id="1">hi</item><empty/></root>',
      "xml"
    )
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.output).toContain('<?xml version="1.0"?>')
    expect(result.output).toContain('<item id="1">hi</item>')
    expect(result.output).toMatch(/<root>\n/)
  })

  it("reports XML syntax errors with line and column", () => {
    const result = formatCode("<root><a></root>", "xml")
    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toMatch(/at line 1, column 16/)
  })

  it("pretty-prints YAML", () => {
    const result = formatCode("foo: bar\nlist: [1, 2]", "yaml")
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.output).toContain("foo: bar")
    expect(result.output).toMatch(/list:\n\s+- 1/)
  })

  it("reports YAML syntax errors with line and column", () => {
    const result = formatCode("a: [", "yaml")
    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toMatch(/at line 2, column 1/)
  })

  it("indents XML when DOMParser is unavailable", () => {
    expect(indentXmlFallback("<root><a>x</a></root>")).toBe(
      `<root>
  <a>x</a>
</root>
`
    )
  })
})
