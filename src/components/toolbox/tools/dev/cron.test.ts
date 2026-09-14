import { describe, expect, it } from "vitest"
import { listCronNextRuns } from "./cron.core"

describe("listCronNextRuns", () => {
  it("lists the next N runs for a 5-field expression in UTC", () => {
    const result = listCronNextRuns("*/5 * * * *", {
      timezone: "UTC",
      count: 3,
      from: new Date("2024-01-01T00:00:00.000Z"),
    })
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.runs).toEqual([
      "2024-01-01 00:05:00 UTC+00:00",
      "2024-01-01 00:10:00 UTC+00:00",
      "2024-01-01 00:15:00 UTC+00:00",
    ])
  })

  it("supports 6-field expressions and time zones", () => {
    const result = listCronNextRuns("0 0 9 * * *", {
      timezone: "Asia/Shanghai",
      count: 2,
      from: new Date("2024-01-01T00:00:00.000Z"),
    })
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.runs[0]).toBe("2024-01-01 09:00:00 UTC+08:00")
    expect(result.runs[1]).toBe("2024-01-02 09:00:00 UTC+08:00")
  })

  it("rejects expressions that are not 5 or 6 fields", () => {
    const result = listCronNextRuns("* * *", {
      timezone: "UTC",
      count: 5,
      from: new Date("2024-01-01T00:00:00.000Z"),
    })
    expect(result).toEqual({
      ok: false,
      error: "Expected 5 or 6 fields, got 3",
    })
  })

  it("surfaces parser errors", () => {
    const result = listCronNextRuns("99 99 * * *", {
      timezone: "UTC",
      count: 1,
      from: new Date("2024-01-01T00:00:00.000Z"),
    })
    expect(result.ok).toBe(false)
  })
})
