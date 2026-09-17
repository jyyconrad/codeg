import { describe, expect, it } from "vitest"

import {
  hasSourcePreviewToggle,
  isBinaryPreviewable,
  isCsvPath,
  isOfficeCliWatchable,
  previewKindFromPath,
  shouldUseFilePreview,
  tabLanguageFromPath,
} from "./file-preview-kind"

describe("previewKindFromPath", () => {
  it.each([
    ["report.pdf", "pdf"],
    ["docs/a.PDF", "pdf"],
    ["letter.docx", "docx"],
    ["deck.pptx", "pptx"],
    ["budget.xlsx", "spreadsheet"],
    ["legacy.xls", "spreadsheet"],
    ["export.csv", "spreadsheet"],
    ["photo.png", "image"],
    ["logo.SVG", "image"],
    ["index.html", "html"],
    ["page.htm", "html"],
    ["README.md", "markdown"],
    ["notes.markdown", "markdown"],
    ["doc.mdx", "source"],
    ["src/app.ts", "source"],
    ["notes.txt", "source"],
  ] as const)("%s -> %s", (path, kind) => {
    expect(previewKindFromPath(path)).toBe(kind)
  })
})

describe("shouldUseFilePreview", () => {
  it("always previews binary office, pdf, images, and excel", () => {
    expect(shouldUseFilePreview("a.pdf", false)).toBe(true)
    expect(shouldUseFilePreview("a.docx", false)).toBe(true)
    expect(shouldUseFilePreview("a.pptx", false)).toBe(true)
    expect(shouldUseFilePreview("a.xlsx", false)).toBe(true)
    expect(shouldUseFilePreview("a.xls", false)).toBe(true)
    expect(shouldUseFilePreview("a.png", false)).toBe(true)
  })

  it("previews markdown, html, and csv only when the source/preview toggle is on", () => {
    expect(shouldUseFilePreview("a.md", true)).toBe(true)
    expect(shouldUseFilePreview("a.md", false)).toBe(false)
    expect(shouldUseFilePreview("a.html", true)).toBe(true)
    expect(shouldUseFilePreview("a.html", false)).toBe(false)
    expect(shouldUseFilePreview("a.csv", true)).toBe(true)
    expect(shouldUseFilePreview("a.csv", false)).toBe(false)
  })

  it("never takes over ordinary source files", () => {
    expect(shouldUseFilePreview("a.ts", true)).toBe(false)
    expect(shouldUseFilePreview("a.mdx", true)).toBe(false)
  })
})

describe("binary / cli / toggle helpers", () => {
  it("marks pdf and excel binaries as binary-previewable, not csv", () => {
    expect(isBinaryPreviewable("a.pdf")).toBe(true)
    expect(isBinaryPreviewable("a.docx")).toBe(true)
    expect(isBinaryPreviewable("a.xlsx")).toBe(true)
    expect(isBinaryPreviewable("a.xls")).toBe(true)
    expect(isBinaryPreviewable("a.csv")).toBe(false)
    expect(isBinaryPreviewable("a.ts")).toBe(false)
  })

  it("treats only docx/xlsx/pptx as officecli-watchable", () => {
    expect(isOfficeCliWatchable("a.docx")).toBe(true)
    expect(isOfficeCliWatchable("a.xlsx")).toBe(true)
    expect(isOfficeCliWatchable("a.pptx")).toBe(true)
    expect(isOfficeCliWatchable("a.pdf")).toBe(false)
    expect(isOfficeCliWatchable("a.xls")).toBe(false)
    expect(isOfficeCliWatchable("a.csv")).toBe(false)
  })

  it("offers a source/preview toggle for markdown, html, and csv", () => {
    expect(hasSourcePreviewToggle("a.md")).toBe(true)
    expect(hasSourcePreviewToggle("a.html")).toBe(true)
    expect(hasSourcePreviewToggle("a.csv")).toBe(true)
    expect(hasSourcePreviewToggle("a.mdx")).toBe(false)
    expect(hasSourcePreviewToggle("a.docx")).toBe(false)
  })

  it("detects csv by extension", () => {
    expect(isCsvPath("a.csv")).toBe(true)
    expect(isCsvPath("a.CSV")).toBe(true)
    expect(isCsvPath("a.xlsx")).toBe(false)
  })
})

describe("tabLanguageFromPath", () => {
  it("stamps synthetic languages onto binary tabs", () => {
    expect(tabLanguageFromPath("a.png")).toBe("image")
    expect(tabLanguageFromPath("a.pdf")).toBe("pdf")
    expect(tabLanguageFromPath("a.docx")).toBe("office")
    expect(tabLanguageFromPath("a.pptx")).toBe("office")
    expect(tabLanguageFromPath("a.xlsx")).toBe("spreadsheet")
    expect(tabLanguageFromPath("a.xls")).toBe("spreadsheet")
  })

  it("keeps csv as a distinct language so the preview toggle can find it", () => {
    expect(tabLanguageFromPath("export.csv")).toBe("csv")
  })
})
