import { describe, expect, it } from "vitest"
import {
  convertUnit,
  formatConversion,
  formatUnitNumber,
  parseUnitInput,
  resolveUnitAlias,
} from "./unit-convert.core"

describe("unit convert", () => {
  it("converts via a canonical unit", () => {
    expect(convertUnit(1, "km", "m")).toBe(1000)
    expect(convertUnit(12, "in", "cm")).toBeCloseTo(30.48, 10)
    expect(convertUnit(1, "MiB", "KiB")).toBe(1024)
  })

  it("applies temperature offsets, not a pure scale", () => {
    expect(convertUnit(0, "°C", "°F")).toBe(32)
    expect(convertUnit(100, "°C", "°F")).toBe(212)
    expect(convertUnit(-40, "°C", "°F")).toBe(-40)
    expect(convertUnit(0, "°C", "K")).toBeCloseTo(273.15, 10)
    expect(convertUnit(32, "°F", "°C")).toBe(0)
    expect(convertUnit(273.15, "K", "°C")).toBeCloseTo(0, 10)
  })

  it("rejects mixed categories", () => {
    expect(() => convertUnit(1, "m", "kg")).toThrow("Incompatible units.")
  })

  it("parses a number with an optional unit alias", () => {
    expect(parseUnitInput("1.5 km")).toEqual({ value: 1.5, unitId: "km" })
    expect(parseUnitInput("32 F")).toEqual({ value: 32, unitId: "°F" })
    expect(parseUnitInput("10")).toEqual({ value: 10 })
    expect(parseUnitInput("nope")).toBeNull()
    expect(resolveUnitAlias("celsius")?.id).toBe("°C")
    expect(formatUnitNumber(0.001)).toBe("0.001")
    expect(formatConversion(1, "km", "m", convertUnit(1, "km", "m"))).toBe(
      "1 km = 1000 m"
    )
  })
})
