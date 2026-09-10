import { describe, expect, it } from "vitest"
import { convertJsonCsv, parseCsv, peekCsvHeaders } from "./json-csv.core"

describe("convertJsonCsv", () => {
  it("maps object arrays to CSV headers in first-seen order", () => {
    const result = convertJsonCsv(
      '[{"name":"Ada","age":36},{"age":1,"city":"Paris"}]',
      { direction: "json-to-csv" }
    )
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.headers).toEqual(["name", "age", "city"])
    expect(result.output).toBe("name,age,city\nAda,36,\n,1,Paris")
  })

  it("quotes commas and embedded quotes", () => {
    const result = convertJsonCsv('[{"note":"a, b \\"c\\""}]', {
      direction: "json-to-csv",
    })
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.output).toBe('note\n"a, b ""c"""')
  })

  it("rejects non-object array items with a JSON path", () => {
    const result = convertJsonCsv("[1]", { direction: "json-to-csv" })
    expect(result).toEqual({
      ok: false,
      error: "Expected an object at path /0",
    })
  })

  it("parses CSV back to JSON using explicit field types", () => {
    const result = convertJsonCsv("name,age,ok\nAda,36,true\nBob,,0", {
      direction: "csv-to-json",
      fieldTypes: { name: "string", age: "number", ok: "boolean" },
    })
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(JSON.parse(result.output)).toEqual([
      { name: "Ada", age: 36, ok: true },
      { name: "Bob", age: null, ok: false },
    ])
  })

  it("reports invalid typed cells with a path", () => {
    const result = convertJsonCsv("age\nx", {
      direction: "csv-to-json",
      fieldTypes: { age: "number" },
    })
    expect(result).toEqual({
      ok: false,
      error: "Invalid number at path /0/age",
    })
  })

  it("parses quoted newlines", () => {
    expect(parseCsv('a\n"b\nc"\n')).toEqual([["a"], ["b\nc"]])
    expect(peekCsvHeaders("x,y\n1,2")).toEqual(["x", "y"])
  })
})
