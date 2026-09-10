export interface TextStats {
  characters: number
  cjk: number
  words: number
  lines: number
  bytes: number
}

const CJK_CHAR =
  /\p{Script=Han}|\p{Script=Hiragana}|\p{Script=Katakana}|\p{Script=Hangul}/u

const encoder = new TextEncoder()

export function countCodePoints(text: string): number {
  return [...text].length
}

export function countCjkChars(text: string): number {
  let count = 0
  for (const ch of text) {
    if (CJK_CHAR.test(ch)) count += 1
  }
  return count
}

export function countWords(text: string): number {
  return text.match(/\S+/g)?.length ?? 0
}

export function countLines(text: string): number {
  if (text === "") return 0
  return text.split(/\r\n|\n|\r/).length
}

export function countUtf8Bytes(text: string): number {
  return encoder.encode(text).length
}

export function computeTextStats(
  text: string,
  countWhitespace: boolean
): TextStats {
  const counted = countWhitespace ? text : text.replace(/\s/g, "")
  return {
    characters: countCodePoints(counted),
    cjk: countCjkChars(text),
    words: countWords(text),
    lines: countLines(text),
    bytes: countUtf8Bytes(counted),
  }
}

export function formatTextStats(stats: TextStats): string {
  return [
    `Characters: ${stats.characters}`,
    `CJK characters: ${stats.cjk}`,
    `Words: ${stats.words}`,
    `Lines: ${stats.lines}`,
    `Bytes (UTF-8): ${stats.bytes}`,
  ].join("\n")
}
