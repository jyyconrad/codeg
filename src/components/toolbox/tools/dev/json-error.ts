export function indexToLineColumn(
  source: string,
  index: number
): { line: number; column: number } {
  const clamped = Math.max(0, Math.min(index, source.length))
  let line = 1
  let column = 1
  for (let i = 0; i < clamped; i++) {
    if (source[i] === "\n") {
      line += 1
      column = 1
    } else {
      column += 1
    }
  }
  return { line, column }
}

export function scanJson(
  text: string
): { ok: true } | { ok: false; index: number } {
  let i = 0
  const n = text.length
  const isDigit = (ch: string | undefined) =>
    ch != null && ch >= "0" && ch <= "9"

  function skipWs() {
    while (i < n) {
      const ch = text[i]
      if (ch !== " " && ch !== "\n" && ch !== "\r" && ch !== "\t") break
      i += 1
    }
  }

  function fail(): never {
    throw i
  }

  function parseString() {
    if (text[i] !== '"') fail()
    i += 1
    while (i < n) {
      const ch = text[i]
      if (ch === '"') {
        i += 1
        return
      }
      if (ch === "\\") {
        i += 1
        if (i >= n) fail()
        const esc = text[i]
        if (esc === "u") {
          i += 1
          for (let k = 0; k < 4; k++) {
            const hex = text[i]
            if (!hex || !/[0-9a-fA-F]/.test(hex)) fail()
            i += 1
          }
          continue
        }
        if (!'"\\/bfnrt'.includes(esc)) fail()
        i += 1
        continue
      }
      if (text.charCodeAt(i) < 0x20) fail()
      i += 1
    }
    fail()
  }

  function parseNumber() {
    if (text[i] === "-") i += 1
    if (text[i] === "0") {
      i += 1
    } else if (isDigit(text[i]) && text[i] !== "0") {
      while (isDigit(text[i])) i += 1
    } else {
      fail()
    }
    if (text[i] === ".") {
      i += 1
      if (!isDigit(text[i])) fail()
      while (isDigit(text[i])) i += 1
    }
    if (text[i] === "e" || text[i] === "E") {
      i += 1
      if (text[i] === "+" || text[i] === "-") i += 1
      if (!isDigit(text[i])) fail()
      while (isDigit(text[i])) i += 1
    }
  }

  function parseLiteral(literal: string) {
    if (text.slice(i, i + literal.length) !== literal) fail()
    i += literal.length
  }

  function parseArray() {
    i += 1
    skipWs()
    if (text[i] === "]") {
      i += 1
      return
    }
    while (true) {
      parseValue()
      skipWs()
      if (text[i] === ",") {
        i += 1
        skipWs()
        continue
      }
      if (text[i] === "]") {
        i += 1
        return
      }
      fail()
    }
  }

  function parseObject() {
    i += 1
    skipWs()
    if (text[i] === "}") {
      i += 1
      return
    }
    while (true) {
      if (text[i] !== '"') fail()
      parseString()
      skipWs()
      if (text[i] !== ":") fail()
      i += 1
      skipWs()
      parseValue()
      skipWs()
      if (text[i] === ",") {
        i += 1
        skipWs()
        continue
      }
      if (text[i] === "}") {
        i += 1
        return
      }
      fail()
    }
  }

  function parseValue() {
    skipWs()
    const ch = text[i]
    if (ch === '"') return parseString()
    if (ch === "{") return parseObject()
    if (ch === "[") return parseArray()
    if (ch === "t") return parseLiteral("true")
    if (ch === "f") return parseLiteral("false")
    if (ch === "n") return parseLiteral("null")
    if (ch === "-" || isDigit(ch)) return parseNumber()
    fail()
  }

  try {
    parseValue()
    skipWs()
    if (i < n) fail()
    return { ok: true }
  } catch (index) {
    return { ok: false, index: typeof index === "number" ? index : i }
  }
}

export function jsonErrorLocation(
  error: unknown,
  source: string
): { line: number; column: number } | null {
  const message = error instanceof Error ? error.message : String(error)
  const lineCol = message.match(/line\s+(\d+)\s+column\s+(\d+)/i)
  if (lineCol) {
    return { line: Number(lineCol[1]), column: Number(lineCol[2]) }
  }
  const pos = message.match(/position\s+(\d+)/i)
  if (pos) return indexToLineColumn(source, Number(pos[1]))
  const scanned = scanJson(source)
  if (!scanned.ok) return indexToLineColumn(source, scanned.index)
  if (/end of (?:JSON|the stream)|Unexpected end/i.test(message)) {
    return indexToLineColumn(source, source.length)
  }
  return null
}

export function formatJsonParseError(error: unknown, source: string): string {
  const message = error instanceof Error ? error.message : String(error)
  const loc = jsonErrorLocation(error, source)
  if (!loc) return message
  return `at line ${loc.line}, column ${loc.column}: ${message}`
}

export function jsonPointer(segments: readonly (string | number)[]): string {
  if (segments.length === 0) return "/"
  return `/${segments
    .map((segment) => String(segment).replace(/~/g, "~0").replace(/\//g, "~1"))
    .join("/")}`
}
