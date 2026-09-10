import { describe, expect, it } from "vitest"
import { convertNaming, tokenizeIdentifier } from "./case-naming-core"

describe("case naming", () => {
  it("tokenizes snake, kebab, and camel boundaries", () => {
    expect(tokenizeIdentifier("hello_world")).toEqual(["hello", "world"])
    expect(tokenizeIdentifier("hello-world")).toEqual(["hello", "world"])
    expect(tokenizeIdentifier("helloWorld")).toEqual(["hello", "world"])
    expect(tokenizeIdentifier("XMLHttpRequest")).toEqual([
      "xml",
      "http",
      "request",
    ])
  })

  it("converts a single identifier to each style", () => {
    const source = "hello_world"
    expect(convertNaming(source, "camelCase")).toBe("helloWorld")
    expect(convertNaming(source, "PascalCase")).toBe("HelloWorld")
    expect(convertNaming(source, "snake_case")).toBe("hello_world")
    expect(convertNaming(source, "kebab-case")).toBe("hello-world")
    expect(convertNaming(source, "CONSTANT_CASE")).toBe("HELLO_WORLD")
  })

  it("converts each line independently and keeps blanks", () => {
    expect(convertNaming("foo-bar\n\nbazQux", "snake_case")).toBe(
      "foo_bar\n\nbaz_qux"
    )
  })

  it("returns empty output for empty input", () => {
    expect(convertNaming("", "camelCase")).toBe("")
    expect(tokenizeIdentifier("   ---  ")).toEqual([])
  })
})
