export type LineSortMode = "keep" | "asc" | "desc"

export interface DedupeSortOptions {
  ignoreCase: boolean
  trimWhitespace: boolean
  sort: LineSortMode
}

export function splitLines(text: string): string[] {
  if (text === "") return []
  return text.split(/\r\n|\n|\r/)
}

export function lineDedupeKey(
  line: string,
  options: Pick<DedupeSortOptions, "ignoreCase" | "trimWhitespace">
): string {
  let key = options.trimWhitespace ? line.trim() : line
  if (options.ignoreCase) key = key.toLowerCase()
  return key
}

export function dedupeSortLines(
  text: string,
  options: DedupeSortOptions
): string {
  const seen = new Set<string>()
  const unique: string[] = []
  for (const line of splitLines(text)) {
    const key = lineDedupeKey(line, options)
    if (seen.has(key)) continue
    seen.add(key)
    unique.push(line)
  }
  if (options.sort === "keep") return unique.join("\n")
  const sorted = [...unique].sort((a, b) => {
    const cmp = lineDedupeKey(a, options).localeCompare(
      lineDedupeKey(b, options)
    )
    return options.sort === "asc" ? cmp : -cmp
  })
  return sorted.join("\n")
}
