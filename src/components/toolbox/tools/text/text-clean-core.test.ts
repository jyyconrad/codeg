import { describe, expect, it } from "vitest"
import { cleanText, toFullWidth, toHalfWidth } from "./text-clean-core"

const off = {
  trim: false,
  dropBlankLines: false,
  normalizeNewlines: false,
  width: "off" as const,
  punct: "off" as const,
}

describe("text clean pipeline", () => {
  it("normalizes newlines, trims, and drops blank lines", () => {
    const input = "  hello  \r\n\r\n\t\r\n  world  \r"
    expect(
      cleanText(input, {
        ...off,
        trim: true,
        dropBlankLines: true,
        normalizeNewlines: true,
      })
    ).toBe("hello\nworld")
  })

  it("converts full width to half width", () => {
    expect(toHalfWidth("ＡＢＣ　１２３")).toBe("ABC 123")
    expect(cleanText("ＡＢＣ", { ...off, width: "full-to-half" })).toBe("ABC")
  })

  it("converts half width to full width", () => {
    expect(toFullWidth("ABC 1")).toBe("ＡＢＣ　１")
    expect(cleanText("ABC", { ...off, width: "half-to-full" })).toBe("ＡＢＣ")
  })

  it("converts CJK and ASCII punctuation", () => {
    expect(cleanText("你好，世界。", { ...off, punct: "cjk-to-ascii" })).toBe(
      "你好,世界."
    )
    expect(cleanText("Hello, world.", { ...off, punct: "ascii-to-cjk" })).toBe(
      "Hello， world。"
    )
  })

  it("applies steps in pipeline order", () => {
    const input = "  Ｈｅｌｌｏ，\r\n\r\n  世界。  "
    expect(
      cleanText(input, {
        trim: true,
        dropBlankLines: true,
        normalizeNewlines: true,
        width: "full-to-half",
        punct: "cjk-to-ascii",
      })
    ).toBe("Hello,\n世界.")
  })

  it("leaves text unchanged when every step is off", () => {
    const input = "  a\r\n\r\nb  "
    expect(cleanText(input, off)).toBe(input)
  })
})
