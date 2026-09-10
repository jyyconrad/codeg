import { describe, expect, it } from "vitest"
import { computeStitchLayout, defaultGridColumns } from "./image-stitch"

const a = { width: 100, height: 50 }
const b = { width: 40, height: 80 }
const c = { width: 100, height: 100 }

describe("stitch layout", () => {
  it("lays out horizontal strips and centers on the max height", () => {
    const layout = computeStitchLayout([a, b], "horizontal", { gap: 10 })
    expect(layout).toEqual({
      width: 150,
      height: 80,
      slots: [
        { x: 0, y: 15, width: 100, height: 50 },
        { x: 110, y: 0, width: 40, height: 80 },
      ],
    })
  })

  it("lays out vertical strips and centers on the max width", () => {
    const layout = computeStitchLayout([a, b], "vertical", { gap: 0 })
    expect(layout.width).toBe(100)
    expect(layout.height).toBe(130)
    expect(layout.slots[1]).toEqual({ x: 30, y: 50, width: 40, height: 80 })
  })

  it("places a grid using equal cells", () => {
    expect(defaultGridColumns(3)).toBe(2)
    const layout = computeStitchLayout([c, c, c], "grid", {
      gap: 8,
      columns: 2,
    })
    expect(layout.width).toBe(208)
    expect(layout.height).toBe(208)
    expect(layout.slots).toEqual([
      { x: 0, y: 0, width: 100, height: 100 },
      { x: 108, y: 0, width: 100, height: 100 },
      { x: 0, y: 108, width: 100, height: 100 },
    ])
  })
})
