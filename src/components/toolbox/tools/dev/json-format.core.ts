import { formatJsonParseError } from "./json-error"

export type JsonFormatMode = "pretty" | "minify"

export type JsonFormatResult =
  | { ok: true; output: string; warning?: string }
  | { ok: false; error: string }

const INTEGER_TOKEN = /^-?\d+$/

export function visitJsonNumberTokens(
  text: string,
  visit: (raw: string, start: number, end: number) => void
): void {
  const n = text.length
  let i = 0
  while (i < n) {
    const c = text[i]
    if (c === '"') {
      i += 1
      while (i < n) {
        if (text[i] === "\\") {
          i += 2
          continue
        }
        if (text[i] === '"') {
          i += 1
          break
        }
        i += 1
      }
      continue
    }
    if (c === "-" || (c >= "0" && c <= "9")) {
      if (isNumberContext(text, i)) {
        const start = i
        if (c === "-") i += 1
        while (i < n && text[i] >= "0" && text[i] <= "9") i += 1
        if (text[i] === ".") {
          i += 1
          while (i < n && text[i] >= "0" && text[i] <= "9") i += 1
        }
        if (text[i] === "e" || text[i] === "E") {
          i += 1
          if (text[i] === "+" || text[i] === "-") i += 1
          while (i < n && text[i] >= "0" && text[i] <= "9") i += 1
        }
        visit(text.slice(start, i), start, i)
        continue
      }
    }
    i += 1
  }
}

function isNumberContext(text: string, index: number): boolean {
  let j = index - 1
  while (
    j >= 0 &&
    (text[j] === " " ||
      text[j] === "\n" ||
      text[j] === "\r" ||
      text[j] === "\t")
  ) {
    j -= 1
  }
  if (j < 0) return true
  const prev = text[j]
  return prev === ":" || prev === "[" || prev === "," || prev === "{"
}

export function integerLosesPrecision(raw: string): boolean {
  if (!INTEGER_TOKEN.test(raw)) return false
  const asNumber = Number(raw)
  if (!Number.isFinite(asNumber)) return true
  try {
    return BigInt(raw) !== BigInt(asNumber)
  } catch {
    return true
  }
}

function collectUnsafeIntegers(text: string): string[] {
  const lost: string[] = []
  visitJsonNumberTokens(text, (raw) => {
    if (integerLosesPrecision(raw)) lost.push(raw)
  })
  return lost
}

function quoteUnsafeIntegers(text: string): string {
  const spans: { start: number; end: number; raw: string }[] = []
  visitJsonNumberTokens(text, (raw, start, end) => {
    if (integerLosesPrecision(raw)) spans.push({ start, end, raw })
  })
  if (spans.length === 0) return text
  let out = ""
  let last = 0
  for (const span of spans) {
    out += `${text.slice(last, span.start)}"${span.raw}"`
    last = span.end
  }
  return out + text.slice(last)
}

function stringifyWarning(lost: string[]): string {
  const first = lost[0]
  const extra = lost.length > 1 ? ` (and ${lost.length - 1} more)` : ""
  return `Integer ${first} lost precision${extra}. Enable stringify big integers.`
}

export function formatJson(
  input: string,
  options: {
    mode: JsonFormatMode
    stringifyBigIntegers: boolean
  }
): JsonFormatResult {
  if (input.trim() === "") return { ok: true, output: "" }
  try {
    JSON.parse(input)
  } catch (error) {
    return { ok: false, error: formatJsonParseError(error, input) }
  }
  const lost = collectUnsafeIntegers(input)
  const source =
    options.stringifyBigIntegers && lost.length > 0
      ? quoteUnsafeIntegers(input)
      : input
  const value = JSON.parse(source) as unknown
  const output = JSON.stringify(
    value,
    null,
    options.mode === "pretty" ? 2 : undefined
  )
  if (!options.stringifyBigIntegers && lost.length > 0) {
    return { ok: true, output, warning: stringifyWarning(lost) }
  }
  return { ok: true, output }
}
