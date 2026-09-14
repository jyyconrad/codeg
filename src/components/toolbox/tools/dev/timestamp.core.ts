import { fromUnixTime, getUnixTime } from "date-fns"
import {
  assertTimeZone,
  formatInTimeZone,
  zonedLocalToUtcMs,
  zonedPartsMatch,
} from "./time-zones"

export type TimestampUnit = "auto" | "seconds" | "milliseconds"

export type TimestampResult =
  | { ok: true; output: string }
  | { ok: false; error: string }

/** JS Date min/max in milliseconds. */
export const MIN_TIMESTAMP_MS = -8640000000000000
export const MAX_TIMESTAMP_MS = 8640000000000000

const NUMERIC = /^-?\d+(?:\.\d+)?$/
const WITH_OFFSET =
  /^(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2})(?::(\d{2})(?:\.(\d{1,3}))?)?(Z|[+-]\d{2}:?\d{2})$/
const NAIVE =
  /^(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}):(\d{2})(?::(\d{2})(?:\.(\d{1,3}))?)?)?$/

function padMs(raw: string | undefined): number {
  if (!raw) return 0
  return Number(raw.padEnd(3, "0").slice(0, 3))
}

function inRange(ms: number): boolean {
  return Number.isFinite(ms) && ms >= MIN_TIMESTAMP_MS && ms <= MAX_TIMESTAMP_MS
}

function rangeError(): { ok: false; error: string } {
  return {
    ok: false,
    error: `Timestamp out of range (must be between ${MIN_TIMESTAMP_MS} and ${MAX_TIMESTAMP_MS} ms)`,
  }
}

type ParsedMs = { ok: true; ms: number } | { ok: false; error: string }

function parseNumericMs(raw: string, unit: TimestampUnit): ParsedMs {
  const absInt = raw.replace(/^-/, "").split(".")[0] ?? ""
  if (absInt.length > 16) return rangeError()

  const value = Number(raw)
  if (!Number.isFinite(value)) return rangeError()

  const resolved: "seconds" | "milliseconds" =
    unit === "auto"
      ? Math.abs(value) >= 1e12
        ? "milliseconds"
        : "seconds"
      : unit

  let ms: number
  if (resolved === "seconds") {
    if (!raw.includes(".")) {
      try {
        const asMs = BigInt(raw) * 1000n
        if (
          asMs < BigInt(MIN_TIMESTAMP_MS) ||
          asMs > BigInt(MAX_TIMESTAMP_MS)
        ) {
          return rangeError()
        }
        ms = Number(asMs)
      } catch {
        return rangeError()
      }
    } else {
      ms = value * 1000
    }
  } else {
    ms = value
  }
  if (!inRange(ms)) return rangeError()
  return { ok: true, ms }
}

function parseDateMs(raw: string, timeZone: string): ParsedMs {
  const offsetMatch = raw.match(WITH_OFFSET)
  if (offsetMatch) {
    const iso = raw.replace(" ", "T")
    const ms = Date.parse(iso)
    if (!Number.isFinite(ms)) {
      return { ok: false, error: "Invalid date" }
    }
    if (!inRange(ms)) return rangeError()
    return { ok: true, ms }
  }

  const naive = raw.match(NAIVE)
  if (naive) {
    const local = {
      year: Number(naive[1]),
      month: Number(naive[2]),
      day: Number(naive[3]),
      hour: Number(naive[4] ?? "0"),
      minute: Number(naive[5] ?? "0"),
      second: Number(naive[6] ?? "0"),
      millisecond: padMs(naive[7]),
    }
    const ms = zonedLocalToUtcMs(local, timeZone)
    if (!inRange(ms)) return rangeError()
    if (
      !zonedPartsMatch(ms, timeZone, {
        year: local.year,
        month: local.month,
        day: local.day,
        hour: local.hour,
        minute: local.minute,
        second: local.second,
      })
    ) {
      return {
        ok: false,
        error: "That local time does not exist in this time zone",
      }
    }
    return { ok: true, ms }
  }

  const parsed = Date.parse(raw)
  if (!Number.isFinite(parsed)) {
    return { ok: false, error: "Invalid date or timestamp" }
  }
  if (!inRange(parsed)) return rangeError()
  return { ok: true, ms: parsed }
}

function formatResult(ms: number, timeZone: string): string {
  const date = new Date(ms)
  const seconds = getUnixTime(date)
  const roundTrip = fromUnixTime(seconds)
  return [
    `Unix seconds: ${seconds}`,
    `Unix milliseconds: ${ms}`,
    `ISO 8601: ${date.toISOString()}`,
    `In ${timeZone}: ${formatInTimeZone(ms, timeZone)}`,
    `Unix seconds as Date: ${roundTrip.toISOString()}`,
  ].join("\n")
}

export function convertTimestamp(
  input: string,
  options: { timeZone: string; unit: TimestampUnit }
): TimestampResult {
  const trimmed = input.trim()
  if (trimmed === "") return { ok: true, output: "" }

  const zone = assertTimeZone(options.timeZone)
  if (!zone.ok) return zone

  const parsed = NUMERIC.test(trimmed)
    ? parseNumericMs(trimmed, options.unit)
    : parseDateMs(trimmed, options.timeZone)

  if (!parsed.ok) return parsed
  return { ok: true, output: formatResult(parsed.ms, options.timeZone) }
}
