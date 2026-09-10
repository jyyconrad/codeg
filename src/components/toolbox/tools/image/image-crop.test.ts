import { describe, expect, it } from "vitest"
import {
  aspectRatioFor,
  clampRect,
  computeResampleSize,
  fitRectToAspect,
  fullImageRect,
  mapDisplayRectToSource,
  mapSourceRectToDisplay,
  normalizeRect,
  roundRect,
} from "./image-crop.core"

describe("crop geometry", () => {
  it("maps a display selection onto source pixels", () => {
    const source = mapDisplayRectToSource(
      { x: 10, y: 20, w: 50, h: 40 },
      0.1,
      1000,
      800
    )
    expect(source).toEqual({ x: 100, y: 200, w: 500, h: 400 })
  })

  it("clamps inverted drags to the image bounds", () => {
    expect(normalizeRect({ x: 40, y: 40, w: -20, h: -10 })).toEqual({
      x: 20,
      y: 30,
      w: 20,
      h: 10,
    })
    const clamped = clampRect({ x: -10, y: -10, w: 500, h: 500 }, 100, 80)
    expect(clamped.x).toBe(0)
    expect(clamped.y).toBe(0)
    expect(clamped.w).toBe(100)
    expect(clamped.h).toBe(80)
  })

  it("locks aspect ratio inside the source", () => {
    const fitted = fitRectToAspect(
      { x: 0, y: 0, w: 100, h: 10 },
      16 / 9,
      160,
      90
    )
    expect(fitted.w / fitted.h).toBeCloseTo(16 / 9, 5)
    expect(fitted.x + fitted.w).toBeLessThanOrEqual(160)
    expect(fitted.y + fitted.h).toBeLessThanOrEqual(90)
    expect(aspectRatioFor("1:1", 100, 50)).toBe(1)
    expect(aspectRatioFor("original", 200, 100)).toBe(2)
    expect(aspectRatioFor("free", 200, 100)).toBeNull()
  })

  it("resamples export size from the source crop", () => {
    const size = computeResampleSize({ x: 10, y: 10, w: 200, h: 100 }, 80)
    expect(size).toEqual({ width: 80, height: 40 })
    expect(fullImageRect(12, 8)).toEqual({ x: 0, y: 0, w: 12, h: 8 })
    expect(roundRect({ x: 1.4, y: 1.6, w: 2.2, h: 0.2 })).toEqual({
      x: 1,
      y: 2,
      w: 2,
      h: 1,
    })
    expect(
      mapSourceRectToDisplay({ x: 100, y: 200, w: 500, h: 400 }, 0.1)
    ).toEqual({ x: 10, y: 20, w: 50, h: 40 })
  })
})
