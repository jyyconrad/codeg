import { describe, expect, it } from "vitest"
import {
  calendarDayDiff,
  diffDates,
  exactDuration,
  formatDateDiff,
  parseDateDiffInput,
  parseToolDate,
  toDateTimeLocalValue,
} from "./date-diff.core"

describe("date-diff", () => {
  it("counts calendar days and optional inclusive endpoints", () => {
    const start = new Date(2024, 0, 1)
    const end = new Date(2024, 0, 3)
    expect(calendarDayDiff(start, end, false)).toBe(2)
    expect(calendarDayDiff(start, end, true)).toBe(3)
    expect(calendarDayDiff(start, start, false)).toBe(0)
    expect(calendarDayDiff(start, start, true)).toBe(1)
    expect(calendarDayDiff(end, start, true)).toBe(3)
  })

  it("reports exact duration independently of calendar days", () => {
    const start = new Date(2024, 0, 1, 0, 0, 0)
    const end = new Date(2024, 0, 2, 12, 0, 0)
    const exact = exactDuration(start, end)
    expect(exact.milliseconds).toBe(36 * 60 * 60 * 1000)
    expect(exact.duration.days).toBe(1)
    expect(exact.duration.hours).toBe(12)
    const reverse = exactDuration(end, start)
    expect(reverse.milliseconds).toBe(-36 * 60 * 60 * 1000)
  })

  it("parses local dates and chained start/end input", () => {
    const date = parseToolDate("2024-01-02")
    expect(date?.getFullYear()).toBe(2024)
    expect(date?.getMonth()).toBe(0)
    expect(date?.getDate()).toBe(2)
    expect(date?.getHours()).toBe(0)
    expect(parseToolDate("2024-13-01")).toBeNull()
    const parsed = parseDateDiffInput("2024-01-01, 2024-12-31")
    expect(parsed?.start.startsWith("2024-01-01T")).toBe(true)
    expect(parsed?.end.startsWith("2024-12-31T")).toBe(true)
    const local = toDateTimeLocalValue(new Date(2024, 5, 7, 8, 9))
    expect(local).toBe("2024-06-07T08:09")
  })

  it("formats calendar days vs duration", () => {
    const start = new Date(2024, 0, 1, 0, 0, 0)
    const end = new Date(2024, 0, 1, 1, 2, 3)
    const text = formatDateDiff(diffDates(start, end, true))
    expect(text).toContain("Calendar days: 1 (inclusive)")
    expect(text).toContain("1 hour 2 minutes 3 seconds")
    expect(text).toContain(`Milliseconds: ${62 * 60 * 1000 + 3000}`)
  })
})
