import { formatJsonParseError, jsonPointer } from "./json-error"

export type JsonCsvDirection = "json-to-csv" | "csv-to-json"
export type CsvFieldType = "string" | "number" | "boolean"

export type JsonCsvResult =
  | { ok: true; output: string; headers: string[] }
  | { ok: false; error: string }

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return (
    value !== null &&
    typeof value === "object" &&
    !Array.isArray(value) &&
    Object.getPrototypeOf(value) === Object.prototype
  )
}

function csvEscape(value: string): string {
  if (/[",\r\n]/.test(value)) return `"${value.replace(/"/g, '""')}"`
  return value
}

function cellFromJson(value: unknown): string {
  if (value === null || value === undefined) return ""
  if (typeof value === "string") return value
  if (typeof value === "number" || typeof value === "boolean") {
    return String(value)
  }
  return JSON.stringify(value)
}

export function parseCsv(text: string): string[][] {
  const source = text.charCodeAt(0) === 0xfeff ? text.slice(1) : text
  const rows: string[][] = []
  let row: string[] = []
  let field = ""
  let i = 0
  let inQuotes = false
  while (i < source.length) {
    const c = source[i]
    if (inQuotes) {
      if (c === '"') {
        if (source[i + 1] === '"') {
          field += '"'
          i += 2
          continue
        }
        inQuotes = false
        i += 1
        continue
      }
      field += c
      i += 1
      continue
    }
    if (c === '"') {
      inQuotes = true
      i += 1
      continue
    }
    if (c === ",") {
      row.push(field)
      field = ""
      i += 1
      continue
    }
    if (c === "\n" || c === "\r") {
      if (c === "\r" && source[i + 1] === "\n") i += 1
      row.push(field)
      field = ""
      rows.push(row)
      row = []
      i += 1
      continue
    }
    field += c
    i += 1
  }
  if (inQuotes) {
    throw new Error("Unterminated quoted CSV field")
  }
  if (field.length > 0 || row.length > 0) {
    row.push(field)
    rows.push(row)
  }
  if (rows.length > 0) {
    const last = rows[rows.length - 1]
    if (last.length === 1 && last[0] === "") rows.pop()
  }
  return rows
}

export function peekCsvHeaders(text: string): string[] {
  try {
    const rows = parseCsv(text)
    return rows[0] ?? []
  } catch {
    return []
  }
}

function collectHeaders(rows: Record<string, unknown>[]): string[] {
  const headers: string[] = []
  const seen = new Set<string>()
  for (const row of rows) {
    for (const key of Object.keys(row)) {
      if (seen.has(key)) continue
      seen.add(key)
      headers.push(key)
    }
  }
  return headers
}

function jsonToCsv(input: string): JsonCsvResult {
  let parsed: unknown
  try {
    parsed = JSON.parse(input) as unknown
  } catch (error) {
    return { ok: false, error: formatJsonParseError(error, input) }
  }
  if (!Array.isArray(parsed)) {
    return { ok: false, error: "Expected a JSON array of objects" }
  }
  const objects: Record<string, unknown>[] = []
  for (let i = 0; i < parsed.length; i++) {
    const item = parsed[i]
    if (!isPlainObject(item)) {
      return {
        ok: false,
        error: `Expected an object at path ${jsonPointer([i])}`,
      }
    }
    objects.push(item)
  }
  const headers = collectHeaders(objects)
  if (headers.length === 0) {
    return { ok: true, output: "", headers }
  }
  const lines = [headers.map(csvEscape).join(",")]
  for (const row of objects) {
    lines.push(
      headers.map((header) => csvEscape(cellFromJson(row[header]))).join(",")
    )
  }
  return { ok: true, output: lines.join("\n"), headers }
}

function coerceCell(
  raw: string,
  type: CsvFieldType,
  path: string
):
  | { ok: true; value: string | number | boolean | null }
  | { ok: false; error: string } {
  if (raw === "") return { ok: true, value: type === "string" ? "" : null }
  if (type === "string") return { ok: true, value: raw }
  if (type === "number") {
    const n = Number(raw)
    if (!Number.isFinite(n)) {
      return { ok: false, error: `Invalid number at path ${path}` }
    }
    return { ok: true, value: n }
  }
  const lower = raw.trim().toLowerCase()
  if (lower === "true" || lower === "1") return { ok: true, value: true }
  if (lower === "false" || lower === "0") return { ok: true, value: false }
  return { ok: false, error: `Invalid boolean at path ${path}` }
}

function csvToJson(
  input: string,
  fieldTypes: Record<string, CsvFieldType>
): JsonCsvResult {
  let rows: string[][]
  try {
    rows = parseCsv(input)
  } catch (error) {
    return {
      ok: false,
      error: error instanceof Error ? error.message : String(error),
    }
  }
  if (rows.length === 0) return { ok: true, output: "[]", headers: [] }
  const headers = rows[0]
  const objects: Record<string, unknown>[] = []
  for (let r = 1; r < rows.length; r++) {
    const row = rows[r]
    const obj: Record<string, unknown> = {}
    for (let c = 0; c < headers.length; c++) {
      const header = headers[c]
      const type = fieldTypes[header] ?? "string"
      const path = jsonPointer([r - 1, header])
      const coerced = coerceCell(row[c] ?? "", type, path)
      if (!coerced.ok) return coerced
      obj[header] = coerced.value
    }
    objects.push(obj)
  }
  return {
    ok: true,
    output: JSON.stringify(objects, null, 2),
    headers,
  }
}

export function convertJsonCsv(
  input: string,
  options: {
    direction: JsonCsvDirection
    fieldTypes?: Record<string, CsvFieldType>
  }
): JsonCsvResult {
  if (input.trim() === "") {
    return { ok: true, output: "", headers: [] }
  }
  if (options.direction === "json-to-csv") return jsonToCsv(input)
  return csvToJson(input, options.fieldTypes ?? {})
}
