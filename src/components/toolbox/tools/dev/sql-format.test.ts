import { describe, expect, it } from "vitest"
import { collapseSqlWhitespace, formatSql } from "./sql-format.core"

describe("formatSql", () => {
  it("pretty-prints a select", () => {
    const result = formatSql("select a,b from t where a=1", {
      mode: "pretty",
      language: "sql",
    })
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.output).toContain("select")
    expect(result.output).toContain("\n")
    expect(result.output).toMatch(/from/)
  })

  it("minifies without touching string literals", () => {
    const sql = "select   'hello  world'   from   t  -- c\nwhere a = 1"
    const result = formatSql(sql, { mode: "minify", language: "sql" })
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.output).toContain("'hello  world'")
    expect(result.output).not.toMatch(/\n/)
    expect(result.output).toBe("select 'hello  world' from t where a = 1")
  })

  it("reports syntax errors with a location", () => {
    const result = formatSql("SELECT FROM $$$", {
      mode: "pretty",
      language: "sql",
    })
    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.error).toMatch(/line 1 column/i)
  })

  it("never executes statements (format-only collapse)", () => {
    expect(collapseSqlWhitespace("delete from t where 1=1")).toBe(
      "delete from t where 1=1"
    )
  })
})
