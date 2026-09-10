export const MAX_REGEX_PATTERN = 500
export const MAX_REGEX_INPUT = 20_000
export const MAX_REGEX_MATCHES = 2_000
const ALLOWED_FLAGS = /^[gimsuyvd]*$/

export type RegexMatchInfo = {
  index: number
  end: number
  text: string
  groups: string[]
  named: Record<string, string>
}

export type RegexTestResult =
  | {
      ok: true
      output: string
      matches: RegexMatchInfo[]
      replaced: string
    }
  | { ok: false; error: string }

function uniqueFlags(flags: string): string {
  const seen = new Set<string>()
  let out = ""
  for (const flag of flags) {
    if (seen.has(flag)) continue
    seen.add(flag)
    out += flag
  }
  return out
}

export function compileRegex(
  pattern: string,
  flags: string
): { ok: true; regex: RegExp } | { ok: false; error: string } {
  if (pattern.length > MAX_REGEX_PATTERN) {
    return {
      ok: false,
      error: `Pattern exceeds ${MAX_REGEX_PATTERN} characters`,
    }
  }
  const cleaned = uniqueFlags(flags.replace(/\s/g, ""))
  if (!ALLOWED_FLAGS.test(cleaned)) {
    return { ok: false, error: `Unsupported flags: ${flags}` }
  }
  try {
    return { ok: true, regex: new RegExp(pattern, cleaned) }
  } catch (error) {
    return {
      ok: false,
      error: error instanceof Error ? error.message : String(error),
    }
  }
}

function namedGroups(match: RegExpExecArray): Record<string, string> {
  const groups = match.groups
  if (!groups) return {}
  const out: Record<string, string> = {}
  for (const [key, value] of Object.entries(groups)) {
    if (value != null) out[key] = value
  }
  return out
}

function formatMatches(matches: RegexMatchInfo[], replaced: string): string {
  if (matches.length === 0) {
    return `Matches (0)\n\nReplace preview:\n${replaced}`
  }
  const blocks = matches.map((match, i) => {
    const lines = [
      `${i}: ${JSON.stringify(match.text)} (${match.index}-${match.end})`,
    ]
    match.groups.forEach((group, g) => {
      lines.push(`  $${g + 1} = ${JSON.stringify(group)}`)
    })
    for (const [name, value] of Object.entries(match.named)) {
      lines.push(`  <${name}> = ${JSON.stringify(value)}`)
    }
    return lines.join("\n")
  })
  return `Matches (${matches.length})\n\n${blocks.join("\n")}\n\nReplace preview:\n${replaced}`
}

export function testRegex(
  input: string,
  options: { pattern: string; flags: string; replacement: string }
): RegexTestResult {
  if (options.pattern === "") {
    return {
      ok: true,
      output: "",
      matches: [],
      replaced: input,
    }
  }
  if (input.length > MAX_REGEX_INPUT) {
    return {
      ok: false,
      error: `Input exceeds ${MAX_REGEX_INPUT} characters`,
    }
  }
  const compiled = compileRegex(options.pattern, options.flags)
  if (!compiled.ok) return compiled

  const globalFlags = compiled.regex.flags.includes("g")
    ? compiled.regex.flags
    : `${compiled.regex.flags}g`
  const scanner = new RegExp(compiled.regex.source, globalFlags)
  const matches: RegexMatchInfo[] = []
  let match: RegExpExecArray | null
  while ((match = scanner.exec(input)) !== null) {
    const text = match[0]
    const index = match.index
    const end = index + text.length
    matches.push({
      index,
      end,
      text,
      groups: match.slice(1).map((group) => group ?? ""),
      named: namedGroups(match),
    })
    if (text === "") {
      scanner.lastIndex = index + 1
    }
    if (matches.length > MAX_REGEX_MATCHES) {
      return {
        ok: false,
        error: `Too many matches (cap ${MAX_REGEX_MATCHES})`,
      }
    }
    if (scanner.lastIndex === index) {
      scanner.lastIndex += 1
    }
  }

  let replaced = input
  try {
    replaced = input.replace(compiled.regex, options.replacement)
  } catch (error) {
    return {
      ok: false,
      error: error instanceof Error ? error.message : String(error),
    }
  }

  return {
    ok: true,
    output: formatMatches(matches, replaced),
    matches,
    replaced,
  }
}
