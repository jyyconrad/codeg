export const DEFAULT_ROW_LIMIT = 100
export const MAX_ROW_LIMIT = 500
export const MIN_ROW_LIMIT = 1
export const DEFAULT_COL_LIMIT = 64
export const MAX_COL_LIMIT = 128
export const MIN_COL_LIMIT = 1

function clampInt(value: number, min: number, max: number, fallback: number): number {
  if (!Number.isFinite(value)) return fallback
  return Math.min(max, Math.max(min, Math.floor(value)))
}

export function clampRowLimit(value: number): number {
  return clampInt(value, MIN_ROW_LIMIT, MAX_ROW_LIMIT, DEFAULT_ROW_LIMIT)
}

export function clampColLimit(value: number): number {
  return clampInt(value, MIN_COL_LIMIT, MAX_COL_LIMIT, DEFAULT_COL_LIMIT)
}

export function clampOffset(value: number): number {
  if (!Number.isFinite(value) || value < 0) return 0
  return Math.floor(value)
}

/** Data rows = total sheet rows minus the sticky header row. */
export function dataRowCount(totalRows: number): number {
  return Math.max(0, totalRows - 1)
}

/**
 * 1-based sheet row numbers for the current data page.
 * Header is row 1 and is not part of the page; the first data row is 2.
 */
export function pageRowRange(
  rowOffset: number,
  rowCountOnPage: number,
  _totalRows: number
): { from: number; to: number } | null {
  if (rowCountOnPage <= 0) return null
  const from = rowOffset + 2
  const to = rowOffset + 1 + rowCountOnPage
  return { from, to }
}

export function nextRowOffset(
  rowOffset: number,
  rowLimit: number,
  totalRows: number
): number {
  const data = dataRowCount(totalRows)
  if (rowOffset + rowLimit >= data) return rowOffset
  return rowOffset + rowLimit
}

export function prevRowOffset(rowOffset: number, rowLimit: number): number {
  return Math.max(0, rowOffset - rowLimit)
}

export function colWindowEnd(
  colOffset: number,
  colLimit: number,
  totalColumns: number
): number {
  return Math.min(colOffset + colLimit, totalColumns)
}

export function nextColOffset(
  colOffset: number,
  colLimit: number,
  totalColumns: number
): number {
  if (colOffset + colLimit >= totalColumns) return colOffset
  return colOffset + colLimit
}

export function prevColOffset(colOffset: number, colLimit: number): number {
  return Math.max(0, colOffset - colLimit)
}
