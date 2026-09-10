import {
  differenceInCalendarDays,
  differenceInMilliseconds,
  intervalToDuration,
  isValid,
  type Duration,
} from "date-fns"

const DATE_ONLY = /^(\d{4})-(\d{2})-(\d{2})$/
const DATETIME_LOCAL = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})(?::(\d{2}))?$/

export type DateDiffResult = {
  calendarDays: number
  milliseconds: number
  duration: Duration
  inclusive: boolean
}

export function parseToolDate(raw: string): Date | null {
  const trimmed = raw.trim()
  if (!trimmed) return null
  const dateOnly = DATE_ONLY.exec(trimmed)
  if (dateOnly) {
    const year = Number(dateOnly[1])
    const month = Number(dateOnly[2])
    const day = Number(dateOnly[3])
    const date = new Date(year, month - 1, day)
    return isValidLocalDate(date, year, month, day) ? date : null
  }
  const local = DATETIME_LOCAL.exec(trimmed)
  if (local) {
    const year = Number(local[1])
    const month = Number(local[2])
    const day = Number(local[3])
    const hour = Number(local[4])
    const minute = Number(local[5])
    const second = local[6] ? Number(local[6]) : 0
    const date = new Date(year, month - 1, day, hour, minute, second)
    return isValid(date) ? date : null
  }
  const iso = new Date(trimmed)
  return isValid(iso) ? iso : null
}

function isValidLocalDate(
  date: Date,
  year: number,
  month: number,
  day: number
): boolean {
  return (
    isValid(date) &&
    date.getFullYear() === year &&
    date.getMonth() === month - 1 &&
    date.getDate() === day
  )
}

export function toDateTimeLocalValue(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0")
  const day = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`
  const time = `${pad(date.getHours())}:${pad(date.getMinutes())}`
  return `${day}T${time}`
}

export function calendarDayDiff(
  start: Date,
  end: Date,
  inclusive: boolean
): number {
  const first = start.getTime() <= end.getTime() ? start : end
  const last = start.getTime() <= end.getTime() ? end : start
  let days = differenceInCalendarDays(last, first)
  if (inclusive) days += 1
  return days
}

export function exactDuration(
  start: Date,
  end: Date
): {
  milliseconds: number
  duration: Duration
} {
  const milliseconds = differenceInMilliseconds(end, start)
  const duration = intervalToDuration({
    start: 0,
    end: Math.abs(milliseconds),
  })
  return { milliseconds, duration }
}

export function diffDates(
  start: Date,
  end: Date,
  inclusive: boolean
): DateDiffResult {
  const { milliseconds, duration } = exactDuration(start, end)
  return {
    calendarDays: calendarDayDiff(start, end, inclusive),
    milliseconds,
    duration,
    inclusive,
  }
}

export function formatDurationParts(duration: Duration): string {
  const parts: string[] = []
  const push = (count: number | undefined, label: string) => {
    if (!count) return
    parts.push(`${count} ${label}${count === 1 ? "" : "s"}`)
  }
  push(duration.years, "year")
  push(duration.months, "month")
  push(duration.days, "day")
  push(duration.hours, "hour")
  push(duration.minutes, "minute")
  push(duration.seconds, "second")
  return parts.length > 0 ? parts.join(" ") : "0 seconds"
}

export function formatDateDiff(result: DateDiffResult): string {
  const sign = result.milliseconds < 0 ? "-" : ""
  return [
    `Calendar days: ${result.calendarDays}${result.inclusive ? " (inclusive)" : ""}`,
    `Duration: ${sign}${formatDurationParts(result.duration)}`,
    `Milliseconds: ${result.milliseconds}`,
  ].join("\n")
}

export function parseDateDiffInput(
  raw: string
): { start: string; end: string } | null {
  const trimmed = raw.trim()
  if (!trimmed) return null
  const lines = trimmed
    .split(/\n+/)
    .map((line) => line.trim())
    .filter(Boolean)
  const tokens =
    lines.length >= 2
      ? lines
      : trimmed
          .split(/\s*(?:,|;|\/|->|—|–)\s*/)
          .map((t) => t.trim())
          .filter(Boolean)
  if (tokens.length >= 2) {
    const startDate = parseToolDate(tokens[0])
    const endDate = parseToolDate(tokens[1])
    if (startDate && endDate) {
      return {
        start: toDateTimeLocalValue(startDate),
        end: toDateTimeLocalValue(endDate),
      }
    }
  }
  const one = parseToolDate(trimmed)
  if (!one) return null
  return {
    start: toDateTimeLocalValue(one),
    end: toDateTimeLocalValue(one),
  }
}
