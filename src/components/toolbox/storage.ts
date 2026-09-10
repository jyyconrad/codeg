import { isToolboxToolId, type ToolboxToolId } from "./types"

const FAVORITES_KEY = "toolbox:favorites"
const RECENT_KEY = "toolbox:recent"
const LAST_TOOL_KEY = "toolbox:last-tool"
const RECENT_LIMIT = 12

function readJsonArray(key: string): string[] {
  if (typeof window === "undefined") return []
  try {
    const raw = localStorage.getItem(key)
    if (!raw) return []
    const parsed = JSON.parse(raw) as unknown
    if (!Array.isArray(parsed)) return []
    return parsed.filter(
      (item): item is string => typeof item === "string" && isToolboxToolId(item)
    )
  } catch {
    return []
  }
}

function writeJson(key: string, value: unknown): void {
  if (typeof window === "undefined") return
  try {
    localStorage.setItem(key, JSON.stringify(value))
  } catch {
    /* quota / private mode */
  }
}

export function loadToolboxFavorites(): ToolboxToolId[] {
  return readJsonArray(FAVORITES_KEY) as ToolboxToolId[]
}

export function saveToolboxFavorites(ids: readonly ToolboxToolId[]): void {
  writeJson(FAVORITES_KEY, ids)
}

export function loadToolboxRecent(): ToolboxToolId[] {
  return readJsonArray(RECENT_KEY) as ToolboxToolId[]
}

export function pushToolboxRecent(id: ToolboxToolId): ToolboxToolId[] {
  const next = [id, ...loadToolboxRecent().filter((item) => item !== id)].slice(
    0,
    RECENT_LIMIT
  )
  writeJson(RECENT_KEY, next)
  return next
}

export function loadLastToolboxTool(): ToolboxToolId | null {
  if (typeof window === "undefined") return null
  try {
    const raw = localStorage.getItem(LAST_TOOL_KEY)
    if (raw && isToolboxToolId(raw)) return raw
    return null
  } catch {
    return null
  }
}

export function saveLastToolboxTool(id: ToolboxToolId | null): void {
  if (typeof window === "undefined") return
  try {
    if (id) localStorage.setItem(LAST_TOOL_KEY, id)
    else localStorage.removeItem(LAST_TOOL_KEY)
  } catch {
    /* ignore */
  }
}
