export interface FindReplaceOptions {
  find: string
  replace: string
  useRegex: boolean
  ignoreCase: boolean
}

export interface ReplaceHit {
  match: string
  replacement: string
  index: number
  line: number
  column: number
}

export interface FindReplaceResult {
  text: string
  hits: ReplaceHit[]
  error: string | null
}

export const FIND_REPLACE_PREVIEW_LIMIT = 100

export function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
}

export function lineColumnAt(
  text: string,
  index: number
): { line: number; column: number } {
  const prefix = text.slice(0, index)
  const lines = prefix.split("\n")
  return {
    line: lines.length,
    column: lines[lines.length - 1].length + 1,
  }
}

function substituteReplacement(
  template: string,
  match: string,
  groups: string[],
  offset: number,
  input: string
): string {
  return template.replace(/\$(\$|&|`|'|\d{1,2})/g, (_, token: string) => {
    if (token === "$") return "$"
    if (token === "&") return match
    if (token === "`") return input.slice(0, offset)
    if (token === "'") return input.slice(offset + match.length)
    const n = Number(token)
    return groups[n - 1] ?? `$${token}`
  })
}

export function runFindReplace(
  text: string,
  options: FindReplaceOptions
): FindReplaceResult {
  if (options.find === "") {
    return { text, hits: [], error: null }
  }

  let pattern: RegExp
  try {
    const source = options.useRegex ? options.find : escapeRegExp(options.find)
    const flags = options.ignoreCase ? "gi" : "g"
    pattern = new RegExp(source, flags)
  } catch (err) {
    const message =
      err instanceof Error ? err.message : "Invalid regular expression"
    return { text, hits: [], error: message }
  }

  const hits: ReplaceHit[] = []
  const replaced = text.replace(pattern, (match, ...rest) => {
    const last = rest[rest.length - 1]
    const hasNamedGroups =
      typeof last === "object" && last !== null && !Array.isArray(last)
    const args = hasNamedGroups ? rest.slice(0, -1) : rest
    const offset = args[args.length - 2] as number
    const groups = args
      .slice(0, -2)
      .map((group) => (typeof group === "string" ? group : ""))
    const replacement = options.useRegex
      ? substituteReplacement(options.replace, match, groups, offset, text)
      : options.replace
    if (hits.length < FIND_REPLACE_PREVIEW_LIMIT) {
      const { line, column } = lineColumnAt(text, offset)
      hits.push({
        match,
        replacement,
        index: offset,
        line,
        column,
      })
    }
    return replacement
  })

  return { text: replaced, hits, error: null }
}
