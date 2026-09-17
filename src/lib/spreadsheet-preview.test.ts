import { describe, expect, it } from "vitest"

import {
  clampColLimit,
  clampRowLimit,
  colWindowEnd,
  dataRowCount,
  DEFAULT_COL_LIMIT,
  DEFAULT_ROW_LIMIT,
  MAX_COL_LIMIT,
  MAX_ROW_LIMIT,
  nextColOffset,
  nextRowOffset,
  pageRowRange,
  prevColOffset,
  prevRowOffset,
} from "./spreadsheet-preview"

describe("clampRowLimit / clampColLimit", () => {
  it("defaults, floors, and clamps to the contract range", () => {
    expect(clampRowLimit(Number.NaN)).toBe(DEFAULT_ROW_LIMIT)
    expect(clampRowLimit(0)).toBe(1)
    expect(clampRowLimit(50)).toBe(50)
    expect(clampRowLimit(1000)).toBe(MAX_ROW_LIMIT)
    expect(clampColLimit(Number.NaN)).toBe(DEFAULT_COL_LIMIT)
    expect(clampColLimit(0)).toBe(1)
    expect(clampColLimit(200)).toBe(MAX_COL_LIMIT)
  })
})

describe("pageRowRange", () => {
  it("reports 1-based sheet rows, skipping the sticky header", () => {
    // 201 rows (1 header + 200 data), first page of 100 data rows: 2–101.
    expect(pageRowRange(0, 100, 201)).toEqual({ from: 2, to: 101 })
    expect(pageRowRange(100, 100, 201)).toEqual({ from: 102, to: 201 })
  })

  it("returns null when there are no data rows on the page", () => {
    expect(pageRowRange(0, 0, 1)).toBeNull()
    expect(pageRowRange(0, 0, 0)).toBeNull()
  })
})

describe("offsets", () => {
  it("counts data rows as totalRows minus the header", () => {
    expect(dataRowCount(201)).toBe(200)
    expect(dataRowCount(1)).toBe(0)
    expect(dataRowCount(0)).toBe(0)
  })

  it("advances and rewinds rowOffset without passing the last data row", () => {
    expect(nextRowOffset(0, 100, 201)).toBe(100)
    expect(nextRowOffset(100, 100, 201)).toBe(100)
    expect(prevRowOffset(100, 100)).toBe(0)
    expect(prevRowOffset(0, 100)).toBe(0)
  })

  it("windows columns the same way", () => {
    expect(colWindowEnd(0, 64, 80)).toBe(64)
    expect(nextColOffset(0, 64, 80)).toBe(64)
    expect(nextColOffset(64, 64, 80)).toBe(64)
    expect(prevColOffset(64, 64)).toBe(0)
  })
})
