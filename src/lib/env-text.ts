/**
 * KEY=VALUE env drafts. Parse ignores comments and blanks. Patch is textual:
 * it rewrites only named keys so comments, blank lines, and a half-typed key
 * survive a structured-panel save.
 */

export function envMapToText(env: Record<string, string>): string {
  return Object.entries(env)
    .map(([key, value]) => `${key}=${value}`)
    .join("\n")
}

export function parseEnvText(envText: string): Record<string, string> {
  const map: Record<string, string> = {}
  for (const rawLine of envText.split(/\r?\n/)) {
    const line = rawLine.trim()
    if (!line || line.startsWith("#")) continue
    const idx = line.indexOf("=")
    if (idx <= 0) continue
    const key = line.slice(0, idx).trim()
    const value = line.slice(idx + 1).trim()
    if (!key) continue
    map[key] = value
  }
  return map
}

/**
 * Set (or, for an empty value, delete) exactly the given keys in a raw env
 * draft, leaving every other LINE byte-identical.
 *
 * Textual on purpose. The obvious implementation — parse to a map, patch,
 * serialize — rewrites the whole textarea, and the parser only understands
 * `KEY=VALUE`: a comment, a blank line, and a half-typed `NEW_PROXY` all
 * vanish. These patches run on refresh and on save completion, so that would
 * silently delete what the user is still typing in the raw editor next to the
 * structured panel that triggered the save.
 *
 * A key appearing on several lines collapses to one (its patched value), which
 * matches how `parseEnvText` reads the draft afterwards.
 */
export function patchEnvText(
  envText: string,
  patch: Record<string, string | undefined>
): string {
  // `key in patch` would also answer yes for `constructor`, `toString` and the
  // rest of Object.prototype — all of them legal env var names — and then read
  // a function where a string was expected. Own properties only.
  const owns = (key: string) => Object.prototype.hasOwnProperty.call(patch, key)
  const pending = new Set(
    Object.keys(patch).filter((key) => (patch[key]?.trim() ?? "") !== "")
  )
  const lines = envText === "" ? [] : envText.split(/\r?\n/)
  const kept: string[] = []
  for (const rawLine of lines) {
    const line = rawLine.trim()
    const idx = line.startsWith("#") ? -1 : line.indexOf("=")
    const key = idx > 0 ? line.slice(0, idx).trim() : ""
    if (!key || !owns(key)) {
      kept.push(rawLine)
      continue
    }
    const value = patch[key]?.trim() ?? ""
    // Empty ⇒ the key is being removed; a duplicate line for a key already
    // emitted goes too, so the result reads back as the value just written.
    if (!value || !pending.delete(key)) continue
    kept.push(`${key}=${value}`)
  }
  if (pending.size > 0) {
    // A key with no line yet goes after the last real one, not after the blank
    // line the user may be about to type into.
    let end = kept.length
    while (end > 0 && kept[end - 1].trim() === "") end -= 1
    const tail = kept.splice(end)
    for (const key of pending) kept.push(`${key}=${patch[key]?.trim() ?? ""}`)
    kept.push(...tail)
  }
  return kept.join("\n")
}
