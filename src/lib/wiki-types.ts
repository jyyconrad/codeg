/** Wire types for the personal wiki settings + workbench shell. */

export const WIKI_DEFAULT_COMPILE_CRON = "0 3 * * *"

export interface WikiCaptureSettings {
  acp_enabled: boolean
  exclude_agent_types: string[]
  exclude_folder_ids: number[]
}

export interface WikiModelPromptSettings {
  model_id: string | null
  prompt: string | null
}

export interface WikiCompileSettings extends WikiModelPromptSettings {
  enabled: boolean
}

/** Persisted settings JSON from the spec. Do not add extra keys. */
export interface WikiSettings {
  enabled: boolean
  vault_path: string | null
  timezone: string
  compile_cron: string
  capture: WikiCaptureSettings
  ingest: WikiModelPromptSettings
  compile: WikiCompileSettings
}

/** GET may also return schedule status that is not written back. */
export interface WikiSettingsView extends WikiSettings {
  next_compile_at?: string | null
  pending_source_count?: number
}

export type WikiJobKind = "ingest" | "compile" | (string & {})

export type WikiJobStatus =
  | "queued"
  | "running"
  | "succeeded"
  | "failed"
  | "cancelled"
  | (string & {})

export interface WikiJob {
  id: string
  kind: WikiJobKind
  status: WikiJobStatus
  attempt?: number | null
  error?: string | null
  error_code?: string | null
  message?: string | null
  created_at?: string | null
  updated_at?: string | null
  started_at?: string | null
  finished_at?: string | null
}

export type WikiSourceKind =
  | "acp-turn"
  | "document"
  | "pasted-text"
  | (string & {})

export type WikiSourceEligibility =
  | "processing"
  | "awaiting-acceptance"
  | "ready"
  | "failed"
  | "cancelled"
  | "withdrawn"
  | (string & {})

export interface WikiSource {
  id: string
  source_kind?: WikiSourceKind | null
  title?: string | null
  source_title?: string | null
  eligibility?: WikiSourceEligibility | null
  captured_at?: string | null
  occurred_at?: string | null
  material_role?: string | null
  personal_role?: string | null
  original_filename?: string | null
  source_path?: string | null
  raw_path?: string | null
}

export interface WikiListPage<T> {
  items: T[]
  total: number
  limit?: number
  offset?: number
}

export interface WikiVaultTreeNode {
  path: string
  name?: string
  kind?: string
  is_dir?: boolean
  children?: WikiVaultTreeNode[]
}

export function systemTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC"
  } catch {
    return "UTC"
  }
}

function emptyToNull(value: string | null | undefined): string | null {
  if (value == null) return null
  const trimmed = value.trim()
  return trimmed.length > 0 ? trimmed : null
}

function asStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return []
  return value.filter((item): item is string => typeof item === "string")
}

function asNumberArray(value: unknown): number[] {
  if (!Array.isArray(value)) return []
  const out: number[] = []
  for (const item of value) {
    const n = typeof item === "number" ? item : Number(item)
    if (Number.isInteger(n)) out.push(n)
  }
  return out
}

export function normalizeWikiSettings(
  raw: Partial<WikiSettingsView> | null | undefined,
  fallbackTimezone = systemTimeZone()
): WikiSettingsView {
  const capture = raw?.capture
  const ingest = raw?.ingest
  const compile = raw?.compile
  return {
    enabled: raw?.enabled === true,
    vault_path: emptyToNull(raw?.vault_path),
    timezone: raw?.timezone?.trim() || fallbackTimezone,
    compile_cron: raw?.compile_cron?.trim() || WIKI_DEFAULT_COMPILE_CRON,
    capture: {
      acp_enabled: capture?.acp_enabled !== false,
      exclude_agent_types: asStringArray(capture?.exclude_agent_types),
      exclude_folder_ids: asNumberArray(capture?.exclude_folder_ids),
    },
    ingest: {
      model_id: emptyToNull(ingest?.model_id),
      prompt: emptyToNull(ingest?.prompt),
    },
    compile: {
      enabled: compile?.enabled !== false,
      model_id: emptyToNull(compile?.model_id),
      prompt: emptyToNull(compile?.prompt),
    },
    next_compile_at: raw?.next_compile_at ?? null,
    pending_source_count:
      typeof raw?.pending_source_count === "number"
        ? raw.pending_source_count
        : undefined,
  }
}

export function wikiSettingsPayload(view: WikiSettingsView): WikiSettings {
  return {
    enabled: view.enabled,
    vault_path: emptyToNull(view.vault_path),
    timezone: view.timezone.trim() || systemTimeZone(),
    compile_cron: view.compile_cron.trim() || WIKI_DEFAULT_COMPILE_CRON,
    capture: {
      acp_enabled: view.capture.acp_enabled,
      exclude_agent_types: view.capture.exclude_agent_types,
      exclude_folder_ids: view.capture.exclude_folder_ids,
    },
    ingest: {
      model_id: emptyToNull(view.ingest.model_id),
      prompt: emptyToNull(view.ingest.prompt),
    },
    compile: {
      enabled: view.compile.enabled,
      model_id: emptyToNull(view.compile.model_id),
      prompt: emptyToNull(view.compile.prompt),
    },
  }
}

export function normalizeWikiList<T extends { id?: string }>(
  payload: unknown
): WikiListPage<T> {
  if (Array.isArray(payload)) {
    return { items: payload as T[], total: payload.length }
  }
  if (payload && typeof payload === "object") {
    const obj = payload as Record<string, unknown>
    for (const key of ["items", "jobs", "sources", "rows", "entries"]) {
      if (Array.isArray(obj[key])) {
        const items = obj[key] as T[]
        const total =
          typeof obj.total === "number"
            ? obj.total
            : typeof obj.count === "number"
              ? obj.count
              : items.length
        return {
          items,
          total,
          limit: typeof obj.limit === "number" ? obj.limit : undefined,
          offset: typeof obj.offset === "number" ? obj.offset : undefined,
        }
      }
    }
  }
  return { items: [], total: 0 }
}

export function normalizeWikiVaultTree(payload: unknown): WikiVaultTreeNode[] {
  if (Array.isArray(payload)) return payload as WikiVaultTreeNode[]
  if (payload && typeof payload === "object") {
    const obj = payload as Record<string, unknown>
    for (const key of ["nodes", "entries", "tree", "children", "items"]) {
      if (Array.isArray(obj[key])) return obj[key] as WikiVaultTreeNode[]
    }
  }
  return []
}

export function wikiVaultReadContent(payload: unknown): string {
  if (typeof payload === "string") return payload
  if (payload && typeof payload === "object") {
    const obj = payload as Record<string, unknown>
    for (const key of ["content", "text", "body"]) {
      if (typeof obj[key] === "string") return obj[key] as string
    }
  }
  return ""
}

export function wikiNodePath(node: WikiVaultTreeNode): string {
  return node.path.replace(/\\/g, "/").replace(/^\.\//, "")
}

export function wikiNodeName(node: WikiVaultTreeNode): string {
  if (node.name && node.name.trim()) return node.name
  const parts = wikiNodePath(node).split("/").filter(Boolean)
  return parts[parts.length - 1] ?? node.path
}

export function wikiNodeIsDir(node: WikiVaultTreeNode): boolean {
  if (typeof node.is_dir === "boolean") return node.is_dir
  const kind = (node.kind ?? "").toLowerCase()
  if (kind === "file") return false
  if (kind === "directory" || kind === "dir" || kind === "folder") {
    return true
  }
  return Array.isArray(node.children)
}

export function vaultNodesUnderPrefix(
  nodes: WikiVaultTreeNode[],
  prefix: string
): WikiVaultTreeNode[] {
  const p = prefix.replace(/\\/g, "/").replace(/\/+$/, "")

  const matches = (path: string): boolean => {
    const n = path.replace(/\\/g, "/").replace(/^\.\//, "")
    return n === p || n.startsWith(`${p}/`)
  }

  const stripPrefix = (path: string): string => {
    const n = path.replace(/\\/g, "/").replace(/^\.\//, "")
    if (n === p) return p
    if (n.startsWith(`${p}/`)) return n
    return n
  }

  const walk = (list: WikiVaultTreeNode[]): WikiVaultTreeNode[] => {
    const out: WikiVaultTreeNode[] = []
    for (const node of list) {
      const path = wikiNodePath(node)
      if (matches(path)) {
        out.push({
          ...node,
          path: stripPrefix(path),
          children: node.children ? walk(node.children) : node.children,
        })
        continue
      }
      if (node.children?.length) {
        const kids = walk(node.children)
        if (kids.length === 1 && wikiNodePath(kids[0]) === p) {
          out.push(...(kids[0].children ?? []))
        } else {
          out.push(...kids)
        }
      }
    }
    return out
  }

  const walked = walk(nodes)
  if (
    walked.length === 1 &&
    wikiNodePath(walked[0]) === p &&
    wikiNodeIsDir(walked[0])
  ) {
    return walked[0].children ?? []
  }
  return walked
}

export function wikiSourceTitle(source: WikiSource): string | null {
  return (
    emptyToNull(source.source_title) ??
    emptyToNull(source.title) ??
    emptyToNull(source.original_filename)
  )
}

export function wikiSourcePreviewPaths(source: WikiSource): string[] {
  const paths: string[] = [`sources/${source.id}.md`]
  const extra = [source.source_path, source.raw_path]
  for (const path of extra) {
    const trimmed = emptyToNull(path)
    if (trimmed && !paths.includes(trimmed)) paths.push(trimmed)
  }
  return paths
}
