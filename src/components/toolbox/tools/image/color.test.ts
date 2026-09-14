import { describe, expect, it } from "vitest"
import {
  colorError,
  formatHex,
  formatHsl,
  formatRgb,
  hslToRgb,
  parseColor,
  parseHex,
  parseHsl,
  parseRgb,
  rgbToHsl,
} from "./color"

describe("parseHex", () => {
  it("parses #RGB, #RRGGBB and alpha variants", () => {
    expect(parseHex("#0f8")).toEqual({ r: 0, g: 255, b: 136, a: 1 })
    expect(parseHex("3388ff")).toEqual({ r: 51, g: 136, b: 255, a: 1 })
    expect(parseHex("#3388FF80")?.a).toBeCloseTo(128 / 255, 5)
  })

  it("rejects out-of-range tokens", () => {
    expect(parseHex("#gg0000")).toBeNull()
    expect(parseHex("#12")).toBeNull()
  })
})

describe("parseRgb / parseHsl", () => {
  it("parses rgb/rgba and validates 0–255", () => {
    expect(parseRgb("rgb(255, 0, 128)")).toEqual({
      r: 255,
      g: 0,
      b: 128,
      a: 1,
    })
    expect(parseRgb("rgba(51, 136, 255, 0.5)")).toEqual({
      r: 51,
      g: 136,
      b: 255,
      a: 0.5,
    })
    expect(parseRgb("255 0 128 / 40%")).toEqual({
      r: 255,
      g: 0,
      b: 128,
      a: 0.4,
    })
    expect(parseRgb("rgb(256, 0, 0)")).toBeNull()
    expect(parseRgb("rgb(-1, 0, 0)")).toBeNull()
    expect(parseRgb("rgba(0, 0, 0, 1.2)")).toBeNull()
  })

  it("parses hsl/hsla and validates ranges", () => {
    expect(parseHsl("hsl(210, 50%, 40%)")).toEqual({
      h: 210,
      s: 50,
      l: 40,
      a: 1,
    })
    expect(parseHsl("hsla(210 50% 40% / 0.25)")).toEqual({
      h: 210,
      s: 50,
      l: 40,
      a: 0.25,
    })
    expect(parseHsl("hsl(0, 101%, 50%)")).toBeNull()
    expect(parseHsl("hsl(361, 50%, 50%)")).toBeNull()
    expect(parseHsl("hsl(0, 50%, -1%)")).toBeNull()
  })
})

describe("color conversion", () => {
  it("round-trips HEX through HSL", () => {
    const parsed = parseHex("#3388FF")
    expect(parsed).toBeTruthy()
    const back = hslToRgb(rgbToHsl(parsed!))
    expect(Math.abs(back.r - parsed!.r)).toBeLessThanOrEqual(1)
    expect(Math.abs(back.g - parsed!.g)).toBeLessThanOrEqual(1)
    expect(Math.abs(back.b - parsed!.b)).toBeLessThanOrEqual(1)
    expect(formatHex(parsed!)).toBe("#3388FF")
  })

  it("keeps alpha in formatted output", () => {
    const color = { r: 51, g: 136, b: 255, a: 0.5 }
    expect(formatRgb(color)).toBe("rgba(51, 136, 255, 0.5)")
    expect(formatHex(color)).toBe("#3388FF80")
    expect(formatHsl(rgbToHsl(color))).toMatch(/^hsla\(/)
  })

  it("detects HEX, RGB, and HSL from a single input", () => {
    expect(parseColor("#fff")?.source).toBe("hex")
    expect(parseColor("rgb(1, 2, 3)")?.source).toBe("rgb")
    expect(parseColor("hsl(10, 20%, 30%)")?.source).toBe("hsl")
    expect(colorError("not a color")).toBe("Invalid HEX / RGB / HSL.")
    expect(colorError("")).toBeNull()
  })
})
