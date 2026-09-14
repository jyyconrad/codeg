import { describe, expect, it } from "vitest"
import { escapeRegExp, runFindReplace } from "./find-replace-core"

describe("find replace", () => {
  it("replaces a literal string and previews each hit", () => {
    const result = runFindReplace("The rain in Spain", {
      find: "ain",
      replace: "oat",
      useRegex: false,
      ignoreCase: false,
    })
    expect(result.error).toBeNull()
    expect(result.text).toBe("The roat in Spoat")
    expect(result.hits).toEqual([
      {
        match: "ain",
        replacement: "oat",
        index: 5,
        line: 1,
        column: 6,
      },
      {
        match: "ain",
        replacement: "oat",
        index: 14,
        line: 1,
        column: 15,
      },
    ])
  })

  it("treats $ as literal in non-regex mode", () => {
    const result = runFindReplace("a a", {
      find: "a",
      replace: "$&",
      useRegex: false,
      ignoreCase: false,
    })
    expect(result.text).toBe("$& $&")
  })

  it("applies regex groups and ignore-case", () => {
    const result = runFindReplace("Foo foo", {
      find: "(f)oo",
      replace: "$1!",
      useRegex: true,
      ignoreCase: true,
    })
    expect(result.error).toBeNull()
    expect(result.text).toBe("F! f!")
    expect(result.hits.map((hit) => hit.match)).toEqual(["Foo", "foo"])
  })

  it("returns an error for an invalid regex and leaves text unchanged", () => {
    const result = runFindReplace("abc", {
      find: "(",
      replace: "x",
      useRegex: true,
      ignoreCase: false,
    })
    expect(result.error).toMatch(/Invalid regular expression/i)
    expect(result.text).toBe("abc")
    expect(result.hits).toEqual([])
  })

  it("is a no-op when find is empty", () => {
    const result = runFindReplace("keep", {
      find: "",
      replace: "x",
      useRegex: true,
      ignoreCase: false,
    })
    expect(result).toEqual({ text: "keep", hits: [], error: null })
  })

  it("escapes regex metacharacters for literal search", () => {
    expect(escapeRegExp("a+b?")).toBe("a\\+b\\?")
    const result = runFindReplace("a+b? a+b?", {
      find: "a+b?",
      replace: "x",
      useRegex: false,
      ignoreCase: false,
    })
    expect(result.text).toBe("x x")
  })
})
