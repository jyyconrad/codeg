import { describe, expect, it } from "vitest"
import { convertZh } from "./zh-convert-core"

describe("zh convert", () => {
  it("converts simplified to traditional at phrase level", () => {
    expect(convertZh("汉字", "s2t")).toBe("漢字")
    expect(convertZh("后面", "s2t")).toBe("後面")
  })

  it("converts traditional to simplified", () => {
    expect(convertZh("漢字", "t2s")).toBe("汉字")
    expect(convertZh("後面", "t2s")).toBe("后面")
  })

  it("returns empty output for empty input", () => {
    expect(convertZh("", "s2t")).toBe("")
  })
})
