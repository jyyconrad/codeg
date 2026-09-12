import { describe, expect, it } from "vitest"

import {
  filterWikiVaultNoise,
  parseWikilink,
  resolveWikiNotePath,
  sanitizeWikilinkPath,
  splitWikiMarkdown,
  vaultNodesUnderPrefix,
  type WikiVaultTreeNode,
} from "./wiki-types"

const sampleTree: WikiVaultTreeNode[] = [
  { path: "index.md", name: "index.md", is_dir: false },
  {
    path: "work",
    name: "work",
    is_dir: true,
    children: [
      { path: "work/index.md", name: "index.md", is_dir: false },
      {
        path: "work/projects",
        name: "projects",
        is_dir: true,
        children: [
          {
            path: "work/projects/alpha.md",
            name: "alpha.md",
            is_dir: false,
          },
        ],
      },
    ],
  },
  {
    path: "raw",
    name: "raw",
    is_dir: true,
    children: [
      { path: "raw/sessions", name: "sessions", is_dir: true, children: [] },
    ],
  },
  {
    path: ".obsidian",
    name: ".obsidian",
    is_dir: true,
    children: [{ path: ".obsidian/app.json", name: "app.json", is_dir: false }],
  },
]

describe("vaultNodesUnderPrefix", () => {
  it("returns the full tree when prefix is empty", () => {
    expect(vaultNodesUnderPrefix(sampleTree, "")).toEqual(sampleTree)
    expect(vaultNodesUnderPrefix(sampleTree, "/")).toEqual(sampleTree)
  })

  it("unwraps a matching directory and keeps nested notes", () => {
    const work = vaultNodesUnderPrefix(sampleTree, "work")
    expect(work.map((node) => node.path)).toEqual([
      "work/index.md",
      "work/projects",
    ])
    expect(work[1]?.children?.[0]?.path).toBe("work/projects/alpha.md")
  })
})

describe("filterWikiVaultNoise", () => {
  it("hides raw and .obsidian by default", () => {
    const filtered = filterWikiVaultNoise(sampleTree)
    expect(filtered.map((node) => node.path)).toEqual(["index.md", "work"])
  })

  it("keeps raw when includeRaw is true", () => {
    const filtered = filterWikiVaultNoise(sampleTree, { includeRaw: true })
    expect(filtered.map((node) => node.path)).toEqual([
      "index.md",
      "work",
      "raw",
    ])
  })
})

describe("wikilink parse", () => {
  it("parses path and aliased label", () => {
    expect(parseWikilink("work/index|Work")).toEqual({
      target: "work/index",
      label: "Work",
    })
    expect(parseWikilink("capabilities/index")).toEqual({
      target: "capabilities/index",
      label: "capabilities/index",
    })
  })

  it("strips .md and heading fragments", () => {
    expect(sanitizeWikilinkPath("work/index.md#Top")).toBe("work/index")
  })

  it("ignores http(s) and parent-dir escapes", () => {
    expect(sanitizeWikilinkPath("https://example.com/note")).toBeNull()
    expect(sanitizeWikilinkPath("http://example.com/note")).toBeNull()
    expect(sanitizeWikilinkPath("../secrets")).toBeNull()
    expect(sanitizeWikilinkPath("/etc/passwd")).toBeNull()
  })

  it("resolves a vault-relative markdown path", () => {
    expect(resolveWikiNotePath("work/index")).toBe("work/index.md")
    expect(resolveWikiNotePath("https://example.com")).toBeNull()
  })

  it("splits markdown into text and wikilink parts", () => {
    const parts = splitWikiMarkdown(
      "See [[work/index|Work]] and [[https://x.test]]."
    )
    expect(parts).toEqual([
      { kind: "text", text: "See " },
      {
        kind: "wikilink",
        raw: "[[work/index|Work]]",
        label: "Work",
        target: "work/index",
      },
      { kind: "text", text: " and " },
      {
        kind: "wikilink",
        raw: "[[https://x.test]]",
        label: "https://x.test",
        target: null,
      },
      { kind: "text", text: "." },
    ])
  })
})
