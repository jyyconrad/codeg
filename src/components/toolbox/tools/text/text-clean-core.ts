export type WidthMode = "off" | "full-to-half" | "half-to-full"
export type PunctMode = "off" | "cjk-to-ascii" | "ascii-to-cjk"

export interface CleanOptions {
  trim: boolean
  dropBlankLines: boolean
  normalizeNewlines: boolean
  width: WidthMode
  punct: PunctMode
}

const CJK_TO_ASCII: readonly [string, string][] = [
  ["……", "..."],
  ["…", "..."],
  ["，", ","],
  ["。", "."],
  ["！", "!"],
  ["？", "?"],
  ["；", ";"],
  ["：", ":"],
  ["（", "("],
  ["）", ")"],
  ["【", "["],
  ["】", "]"],
  ["『", '"'],
  ["』", '"'],
  ["「", '"'],
  ["」", '"'],
  ["“", '"'],
  ["”", '"'],
  ["‘", "'"],
  ["’", "'"],
  ["、", ","],
  ["《", "<"],
  ["》", ">"],
  ["—", "-"],
  ["–", "-"],
  ["～", "~"],
]

const ASCII_TO_CJK: readonly [string, string][] = [
  ["...", "…"],
  [",", "，"],
  [".", "。"],
  ["!", "！"],
  ["?", "？"],
  [";", "；"],
  [":", "："],
  ["(", "（"],
  [")", "）"],
  ["[", "【"],
  ["]", "】"],
  ['"', "“"],
  ["'", "‘"],
  ["<", "《"],
  [">", "》"],
  ["~", "～"],
]

function splitLines(text: string): string[] {
  if (text === "") return []
  return text.split(/\r\n|\n|\r/)
}

function applyPairs(text: string, pairs: readonly [string, string][]): string {
  const ordered = [...pairs].sort((a, b) => b[0].length - a[0].length)
  let out = text
  for (const [from, to] of ordered) {
    out = out.split(from).join(to)
  }
  return out
}

export function toHalfWidth(text: string): string {
  let out = ""
  for (const ch of text) {
    const code = ch.codePointAt(0)!
    if (code === 0x3000) {
      out += " "
    } else if (code >= 0xff01 && code <= 0xff5e) {
      out += String.fromCodePoint(code - 0xfee0)
    } else {
      out += ch
    }
  }
  return out
}

export function toFullWidth(text: string): string {
  let out = ""
  for (const ch of text) {
    const code = ch.codePointAt(0)!
    if (code === 0x20) {
      out += "\u3000"
    } else if (code >= 0x21 && code <= 0x7e) {
      out += String.fromCodePoint(code + 0xfee0)
    } else {
      out += ch
    }
  }
  return out
}

export function cleanText(text: string, options: CleanOptions): string {
  let out = text
  if (options.normalizeNewlines) {
    out = out.replace(/\r\n|\r/g, "\n")
  }
  if (options.trim) {
    out = splitLines(out)
      .map((line) => line.trim())
      .join("\n")
      .trim()
  }
  if (options.dropBlankLines) {
    out = splitLines(out)
      .filter((line) => line.trim() !== "")
      .join("\n")
  }
  if (options.width === "full-to-half") out = toHalfWidth(out)
  else if (options.width === "half-to-full") out = toFullWidth(out)
  if (options.punct === "cjk-to-ascii") out = applyPairs(out, CJK_TO_ASCII)
  else if (options.punct === "ascii-to-cjk") out = applyPairs(out, ASCII_TO_CJK)
  return out
}
