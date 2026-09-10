export const COMMON_TIMEZONES = [
  "UTC",
  "Asia/Shanghai",
  "Asia/Tokyo",
  "Asia/Hong_Kong",
  "Asia/Singapore",
  "Asia/Kolkata",
  "Europe/London",
  "Europe/Paris",
  "Europe/Berlin",
  "Europe/Moscow",
  "America/New_York",
  "America/Chicago",
  "America/Denver",
  "America/Los_Angeles",
  "America/Sao_Paulo",
  "Australia/Sydney",
  "Pacific/Auckland",
] as const

export function resolveLocalTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC"
  } catch {
    return "UTC"
  }
}

export function listToolTimezones(): { value: string; label: string }[] {
  const local = resolveLocalTimeZone()
  const seen = new Set<string>()
  const items: { value: string; label: string }[] = []
  const add = (value: string, label: string) => {
    if (seen.has(value)) return
    seen.add(value)
    items.push({ value, label })
  }
  add("UTC", "UTC")
  add(local, `Local (${local})`)
  for (const zone of COMMON_TIMEZONES) add(zone, zone)
  return items
}

export function assertTimeZone(
  timeZone: string
): { ok: true } | { ok: false; error: string } {
  try {
    new Intl.DateTimeFormat("en-US", { timeZone })
    return { ok: true }
  } catch {
    return { ok: false, error: `Unknown time zone: ${timeZone}` }
  }
}

type DateParts = {
  year: string
  month: string
  day: string
  hour: string
  minute: string
  second: string
  timeZoneName?: string
}

function zonedParts(utcMs: number, timeZone: string): DateParts {
  const dtf = new Intl.DateTimeFormat("en-US", {
    timeZone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
    timeZoneName: "shortOffset",
  })
  const map: Partial<DateParts> = {}
  for (const part of dtf.formatToParts(new Date(utcMs))) {
    if (part.type !== "literal") {
      map[part.type as keyof DateParts] = part.value
    }
  }
  return {
    year: map.year ?? "0000",
    month: map.month ?? "01",
    day: map.day ?? "01",
    hour: map.hour ?? "00",
    minute: map.minute ?? "00",
    second: map.second ?? "00",
    timeZoneName: map.timeZoneName,
  }
}

function utcOffsetLabel(utcMs: number, timeZone: string): string {
  const parts = zonedParts(utcMs, timeZone)
  const asUtc = Date.UTC(
    Number(parts.year),
    Number(parts.month) - 1,
    Number(parts.day),
    Number(parts.hour),
    Number(parts.minute),
    Number(parts.second),
    0
  )
  const truncated = Math.trunc(utcMs / 1000) * 1000
  const offsetMin = Math.round((asUtc - truncated) / 60000)
  const sign = offsetMin >= 0 ? "+" : "-"
  const abs = Math.abs(offsetMin)
  const hh = String(Math.floor(abs / 60)).padStart(2, "0")
  const mm = String(abs % 60).padStart(2, "0")
  return `UTC${sign}${hh}:${mm}`
}

export function formatInTimeZone(utcMs: number, timeZone: string): string {
  const parts = zonedParts(utcMs, timeZone)
  return `${parts.year}-${parts.month}-${parts.day} ${parts.hour}:${parts.minute}:${parts.second} ${utcOffsetLabel(utcMs, timeZone)}`
}

export function zonedLocalToUtcMs(
  local: {
    year: number
    month: number
    day: number
    hour: number
    minute: number
    second: number
    millisecond: number
  },
  timeZone: string
): number {
  const wanted = Date.UTC(
    local.year,
    local.month - 1,
    local.day,
    local.hour,
    local.minute,
    local.second,
    0
  )
  let utc = wanted
  for (let i = 0; i < 4; i++) {
    const shown = zonedParts(utc, timeZone)
    const asUtc = Date.UTC(
      Number(shown.year),
      Number(shown.month) - 1,
      Number(shown.day),
      Number(shown.hour),
      Number(shown.minute),
      Number(shown.second),
      0
    )
    utc -= asUtc - wanted
  }
  return utc + local.millisecond
}

export function zonedPartsMatch(
  utcMs: number,
  timeZone: string,
  local: {
    year: number
    month: number
    day: number
    hour: number
    minute: number
    second: number
  }
): boolean {
  const shown = zonedParts(utcMs, timeZone)
  return (
    Number(shown.year) === local.year &&
    Number(shown.month) === local.month &&
    Number(shown.day) === local.day &&
    Number(shown.hour) === local.hour &&
    Number(shown.minute) === local.minute &&
    Number(shown.second) === local.second
  )
}
