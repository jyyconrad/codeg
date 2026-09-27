import { parseLocalFileTarget } from "./link-classify"

/**
 * Classifies a markdown link / file address into a small set of "resource
 * kinds", purely for presentation: choosing the type icon shown before the
 * address in chat messages.
 *
 * File detection is `parseLocalFileTarget`, the same function the click
 * handler uses, so a badge is never tagged with a type the opener refuses.
 * That includes workspace-relative paths (`src/main.rs`, `docs/手册.docx`)
 * and a bare document filename (`README.md`). Host-like `www.example.com`
 * and unknown schemes stay untagged.
 *
 *   - `file`  → opened in the workspace file panel
 *   - `web`   → opened in the browser (http / https, or protocol-relative //)
 *   - `email` → mailto:
 *   - `phone` → tel:
 */

export type ResourceKind = "file" | "web" | "email" | "phone"

const URL_SCHEME = /^([a-zA-Z][a-zA-Z\d+\-.]*):/

export function classifyResourceKind(rawUrl: string): ResourceKind | null {
  const trimmed = rawUrl.trim()
  if (!trimmed) return null

  if (parseLocalFileTarget(trimmed)) return "file"

  const scheme = trimmed.match(URL_SCHEME)?.[1]?.toLowerCase()
  if (scheme) {
    if (scheme === "mailto") return "email"
    if (scheme === "tel") return "phone"
    if (scheme === "http" || scheme === "https") return "web"
    // Unknown / unsupported scheme — the click handler can't open it, so don't
    // imply a type.
    return null
  }

  // Protocol-relative "//host/path" routes to the browser.
  if (trimmed.startsWith("//")) return "web"

  return null
}
