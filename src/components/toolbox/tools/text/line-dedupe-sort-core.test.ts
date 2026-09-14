import { describe, expect, it } from "vitest"
import { dedupeSortLines } from "./line-dedupe-sort-core"

describe("line dedupe and sort", () => {
  it("keeps first occurrence when order is preserved", () => {
    expect(
      dedupeSortLines("b\na\nb\nc", {
        ignoreCase: false,
        trimWhitespace: false,
        sort: "keep",
      })
    ).toBe("b\na\nc")
  })

  it("dedupes with case and whitespace keys", () => {
    expect(
      dedupeSortLines("Banana\n  banana  \napple", {
        ignoreCase: true,
        trimWhitespace: true,
        sort: "keep",
      })
    ).toBe("Banana\napple")
  })

  it("sorts unique lines by the same key", () => {
    expect(
      dedupeSortLines("Banana\napple\nCherry", {
        ignoreCase: true,
        trimWhitespace: true,
        sort: "asc",
      })
    ).toBe("apple\nBanana\nCherry")
    expect(
      dedupeSortLines("b\na\nc", {
        ignoreCase: false,
        trimWhitespace: false,
        sort: "desc",
      })
    ).toBe("c\nb\na")
  })

  it("returns empty output for empty input", () => {
    expect(
      dedupeSortLines("", {
        ignoreCase: true,
        trimWhitespace: true,
        sort: "asc",
      })
    ).toBe("")
  })
})
