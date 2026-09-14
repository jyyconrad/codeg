import { EncodingError } from "./error"

export type UnicodeStyle = "unicode" | "json"

export function encodeUnicode(text: string, style: UnicodeStyle): string {
  if (text === "") return ""
  if (style === "json") return encodeJsonEscapes(text)
  return encodeUnicodeEscapes(text)
}

export function decodeUnicode(text: string, style: UnicodeStyle): string {
  if (text === "") return ""
  if (style === "json") return decodeJsonEscapes(text)
  return decodeUnicodeEscapes(text)
}

export function encodeUnicodeEscapes(text: string): string {
  let out = ""
  for (let i = 0; i < text.length; i++) {
    const code = text.charCodeAt(i)
    if (code < 0x80) {
      out += text[i]
    } else {
      out += `\\u${code.toString(16).padStart(4, "0")}`
    }
  }
  return out
}

export function decodeUnicodeEscapes(text: string): string {
  let out = ""
  for (let i = 0; i < text.length; i++) {
    const ch = text[i]
    if (ch !== "\\") {
      out += ch
      continue
    }
    if (text[i + 1] !== "u") {
      throw new EncodingError("Invalid Unicode escape")
    }
    const hex = text.slice(i + 2, i + 6)
    if (hex.length !== 4 || /[^0-9a-fA-F]/.test(hex)) {
      throw new EncodingError("Invalid Unicode escape")
    }
    out += String.fromCharCode(Number.parseInt(hex, 16))
    i += 5
  }
  return out
}

export function encodeJsonEscapes(text: string): string {
  const json = JSON.stringify(text).slice(1, -1)
  return json.replace(/[\u007f-\uffff]/g, (ch) => {
    return `\\u${ch.charCodeAt(0).toString(16).padStart(4, "0")}`
  })
}

export function decodeJsonEscapes(text: string): string {
  const trimmed = text.trim()
  try {
    const wrapped =
      trimmed.startsWith('"') && trimmed.endsWith('"') && trimmed.length >= 2
        ? trimmed
        : `"${trimmed}"`
    const parsed: unknown = JSON.parse(wrapped)
    if (typeof parsed !== "string") {
      throw new EncodingError("Invalid JSON escape")
    }
    return parsed
  } catch (error) {
    if (error instanceof EncodingError) throw error
    throw new EncodingError("Invalid JSON escape")
  }
}
