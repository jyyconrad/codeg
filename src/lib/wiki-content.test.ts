import { describe, expect, it } from "vitest"
import {
  prepareWikiMarkdown,
  resolveWikiLink,
  wikiBodyForReading,
} from "./wiki-content"

describe("Wiki reading contract", () => {
  it("keeps encoded filename characters separate from heading anchors", () => {
    expect(resolveWikiLink("a%23b.md#结果", "work/index.md")).toEqual({
      path: "work/a#b.md",
      anchor: "结果",
    })
  })
  it("removes only valid opening YAML and leaves malformed material readable", () => {
    expect(prepareWikiMarkdown("---\ntitle: Read\n---\n# Body")).toEqual({
      body: "# Body",
      warning: false,
    })
    expect(prepareWikiMarkdown("---\ntitle: [broken\n---\nKeep me")).toEqual({
      body: "---\ntitle: [broken\n---\nKeep me",
      warning: true,
    })
    expect(
      prepareWikiMarkdown("---\ntitle: Missing end\nKeep me").body
    ).toContain("Keep me")
  })
  it("resolves relative, library and same-page links without leaving the library", () => {
    expect(
      resolveWikiLink("../methods/check.md#结果", "work/projects/one.md")
    ).toEqual({ path: "work/methods/check.md", anchor: "结果" })
    expect(
      resolveWikiLink("wiki:capabilities/design#Checks", "work/one.md")
    ).toEqual({ path: "capabilities/design.md", anchor: "checks" })
    expect(resolveWikiLink("#检查清单", "work/one.md")).toEqual({
      path: "work/one.md",
      anchor: "检查清单",
    })
    for (const target of [
      "../../secret",
      "%2e%2e/%2e%2e/secret",
      "/etc/passwd",
      "javascript:alert(1)",
      "file:///tmp/a",
      "../%5csecret",
    ])
      expect(resolveWikiLink(target, "work/one.md")).toBeNull()
  })
})

it("reads evidence line ranges against the original document including YAML", async () => {
  const { wikiSourceLineExcerpt } = await import("./wiki-content")
  expect(
    wikiSourceLineExcerpt(
      "---\ntitle: Heading\n---\nFirst body line\nSecond body line",
      "#L4-L5"
    )
  ).toEqual({ start: 4, end: 5, text: "First body line\nSecond body line" })
  expect(wikiSourceLineExcerpt("one\ntwo", "#L99-L100")).toBeNull()
})

it("shows the title once while retaining other headings and code", () => {
  expect(wikiBodyForReading("# 标题\n\n## 结论\n正文", "标题")).toBe(
    "## 结论\n正文"
  )
  expect(wikiBodyForReading("# 另一标题\n正文", "标题")).toBe(
    "# 另一标题\n正文"
  )
  expect(wikiBodyForReading("```md\n# 标题\n```", "标题")).toContain("# 标题")
})
