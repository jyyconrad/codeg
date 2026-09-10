import { describe, expect, it } from "vitest"
import {
  computeTextStats,
  countCjkChars,
  countCodePoints,
  countLines,
  countUtf8Bytes,
  countWords,
  formatTextStats,
} from "./text-stats-core"

describe("text stats", () => {
  it("returns zeros for empty input", () => {
    expect(computeTextStats("", true)).toEqual({
      characters: 0,
      cjk: 0,
      words: 0,
      lines: 0,
      bytes: 0,
    })
  })

  it("counts characters, words, lines, and utf-8 bytes", () => {
    const text = "Hi 你\n"
    expect(countCodePoints(text)).toBe(5)
    expect(countCjkChars(text)).toBe(1)
    expect(countWords(text)).toBe(2)
    expect(countLines(text)).toBe(2)
    expect(countUtf8Bytes(text)).toBe(7)
  })

  it("can exclude whitespace from character and byte counts", () => {
    const text = "a b\n中"
    const withWs = computeTextStats(text, true)
    const withoutWs = computeTextStats(text, false)
    expect(withWs.characters).toBe(5)
    expect(withoutWs.characters).toBe(3)
    expect(withoutWs.bytes).toBe(countUtf8Bytes("ab中"))
    expect(withoutWs.cjk).toBe(1)
    expect(withoutWs.words).toBe(3)
    expect(withoutWs.lines).toBe(2)
  })

  it("treats a trailing newline as an extra line", () => {
    expect(countLines("a\n")).toBe(2)
    expect(countWords("   ")).toBe(0)
  })

  it("formats a copyable summary", () => {
    expect(
      formatTextStats({
        characters: 1,
        cjk: 2,
        words: 3,
        lines: 4,
        bytes: 5,
      })
    ).toBe(
      [
        "Characters: 1",
        "CJK characters: 2",
        "Words: 3",
        "Lines: 4",
        "Bytes (UTF-8): 5",
      ].join("\n")
    )
  })
})
