import { describe, expect, it } from "vitest"
import { diffTexts } from "./text-diff-core"

describe("text diff", () => {
  it("returns no rows for two empty strings", () => {
    expect(diffTexts("", "")).toEqual({ rows: [], unified: "" })
  })

  it("marks added and removed lines", () => {
    const { unified, rows } = diffTexts("keep\ngone", "keep\nnew")
    expect(unified).toContain(" keep")
    expect(unified).toContain("-gone")
    expect(unified).toContain("+new")
    expect(rows.some((row) => row.type === "replace")).toBe(true)
  })

  it("adds inline marks on a replaced line", () => {
    const { rows } = diffTexts("hello", "hallo")
    expect(rows).toHaveLength(1)
    const row = rows[0]
    expect(row.type).toBe("replace")
    if (row.type !== "replace") return
    expect(row.parts.map((part) => part.type)).toContain("remove")
    expect(row.parts.map((part) => part.type)).toContain("add")
    expect(row.before).toBe("hello")
    expect(row.after).toBe("hallo")
  })
})
