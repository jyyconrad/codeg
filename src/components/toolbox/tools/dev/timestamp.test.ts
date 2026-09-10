import { describe, expect, it } from "vitest"
import {
  convertTimestamp,
  MAX_TIMESTAMP_MS,
  MIN_TIMESTAMP_MS,
} from "./timestamp.core"

describe("convertTimestamp", () => {
  it("converts unix seconds to a zoned date", () => {
    const result = convertTimestamp("1700000000", {
      timeZone: "Asia/Shanghai",
      unit: "seconds",
    })
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.output).toContain("Unix seconds: 1700000000")
    expect(result.output).toContain("Unix milliseconds: 1700000000000")
    expect(result.output).toContain("2023-11-14T22:13:20.000Z")
    expect(result.output).toContain(
      "In Asia/Shanghai: 2023-11-15 06:13:20 UTC+08:00"
    )
  })

  it("auto-detects millisecond timestamps", () => {
    const result = convertTimestamp("1700000000000", {
      timeZone: "UTC",
      unit: "auto",
    })
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.output).toContain("Unix seconds: 1700000000")
    expect(result.output).toContain("In UTC: 2023-11-14 22:13:20 UTC+00:00")
  })

  it("interprets naive datetimes in the selected time zone", () => {
    const result = convertTimestamp("2023-11-15 06:13:20", {
      timeZone: "Asia/Shanghai",
      unit: "auto",
    })
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.output).toContain("Unix seconds: 1700000000")
  })

  it("parses ISO instants with an explicit offset", () => {
    const result = convertTimestamp("2023-11-14T22:13:20Z", {
      timeZone: "America/New_York",
      unit: "auto",
    })
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.output).toContain("Unix seconds: 1700000000")
    expect(result.output).toMatch(/In America\/New_York: 2023-11-14 17:13:20/)
  })

  it("rejects timestamps outside the JS Date range", () => {
    const tooBig = String(MAX_TIMESTAMP_MS + 1)
    const tooSmall = String(MIN_TIMESTAMP_MS - 1)
    const high = convertTimestamp(tooBig, {
      timeZone: "UTC",
      unit: "milliseconds",
    })
    const low = convertTimestamp(tooSmall, {
      timeZone: "UTC",
      unit: "milliseconds",
    })
    expect(high.ok).toBe(false)
    expect(low.ok).toBe(false)
    if (high.ok || low.ok) return
    expect(high.error).toMatch(/out of range/)
    expect(low.error).toMatch(/out of range/)
  })

  it("rejects unknown time zones", () => {
    const result = convertTimestamp("0", {
      timeZone: "Mars/Phobos",
      unit: "seconds",
    })
    expect(result).toEqual({
      ok: false,
      error: "Unknown time zone: Mars/Phobos",
    })
  })

  it("rejects invalid date strings", () => {
    const result = convertTimestamp("not a date", {
      timeZone: "UTC",
      unit: "auto",
    })
    expect(result.ok).toBe(false)
  })
})
