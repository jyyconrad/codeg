import { describe, expect, it } from "vitest"
import {
  clampAngle,
  clampOpacity,
  coverDiagonal,
  iterTileOrigins,
} from "./image-watermark.core"

describe("watermark tiling", () => {
  it("steps by stamp size plus spacing", () => {
    expect(iterTileOrigins(100, 50, 40, 20, 10)).toEqual([
      { x: 0, y: 0 },
      { x: 50, y: 0 },
      { x: 0, y: 30 },
      { x: 50, y: 30 },
    ])
  })

  it("covers a rotated canvas with the diagonal", () => {
    expect(coverDiagonal(3, 4)).toBe(5)
  })

  it("clamps opacity and angle", () => {
    expect(clampOpacity(1.4)).toBe(1)
    expect(clampOpacity(-0.2)).toBe(0)
    expect(clampAngle(200)).toBe(180)
    expect(clampAngle(-200)).toBe(-180)
  })
})
