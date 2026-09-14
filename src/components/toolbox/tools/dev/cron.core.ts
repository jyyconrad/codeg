import { CronExpressionParser } from "cron-parser"
import { assertTimeZone, formatInTimeZone } from "./time-zones"

export const CRON_MIN_COUNT = 1
export const CRON_MAX_COUNT = 50

export type CronResult =
  | { ok: true; output: string; runs: string[] }
  | { ok: false; error: string }

export function listCronNextRuns(
  expression: string,
  options: { timezone: string; count: number; from?: Date }
): CronResult {
  const trimmed = expression.trim()
  if (trimmed === "") return { ok: true, output: "", runs: [] }

  const fields = trimmed.split(/\s+/)
  if (fields.length < 5 || fields.length > 6) {
    return {
      ok: false,
      error: `Expected 5 or 6 fields, got ${fields.length}`,
    }
  }

  const count = Math.trunc(options.count)
  if (
    !Number.isFinite(count) ||
    count < CRON_MIN_COUNT ||
    count > CRON_MAX_COUNT
  ) {
    return {
      ok: false,
      error: `Count must be between ${CRON_MIN_COUNT} and ${CRON_MAX_COUNT}`,
    }
  }

  const zone = assertTimeZone(options.timezone)
  if (!zone.ok) return zone

  try {
    const parsed = CronExpressionParser.parse(trimmed, {
      tz: options.timezone,
      currentDate: options.from ?? new Date(),
    })
    const runs = parsed
      .take(count)
      .map((date) =>
        formatInTimeZone(date.toDate().getTime(), options.timezone)
      )
    return { ok: true, output: runs.join("\n"), runs }
  } catch (error) {
    return {
      ok: false,
      error: error instanceof Error ? error.message : String(error),
    }
  }
}
