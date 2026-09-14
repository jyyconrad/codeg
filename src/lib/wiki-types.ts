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

export interface WikiSynthesizeSettings extends WikiModelPromptSettings {
  enabled: boolean
}

/** Same shape as synthesize; kept for existing re-exports. */
export type WikiCompileSettings = WikiSynthesizeSettings

/** Persisted settings JSON from the spec. Do not add extra keys. */
export interface WikiSettings {
  enabled: boolean
  vault_path: string | null
  timezone: string
  compile_cron: string
  capture: WikiCaptureSettings
  turn_summary: WikiModelPromptSettings
  session_rollup: WikiModelPromptSettings
  synthesize: WikiSynthesizeSettings
}

/** GET may also return schedule status that is not written back. */
export interface WikiSettingsView extends WikiSettings {
  next_compile_at?: string | null
  pending_source_count?: number
  turn_summary_builtin_prompt?: string | null
  session_rollup_builtin_prompt?: string | null
  synthesize_builtin_prompt?: string | null
}

/** Incoming GET/PATCH may still carry retired ingest/compile slots. */
export type WikiSettingsIncoming = Partial<WikiSettingsView> & {
  ingest?: Partial<WikiModelPromptSettings> | null
  compile?: Partial<WikiSynthesizeSettings> | null
}

export type WikiJobKind =
  | "turn_summary"
  | "session_rollup"
  | "wiki_synthesize"
  | "ingest"
  | "compile"
  | (string & {})

export type WikiJobKindKey =
  | "turn_summary"
  | "session_rollup"
  | "wiki_synthesize"
  | "ingest"
  | "compile"
  | "unknown"

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
  error_message?: string | null
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

export type WikiMaterialRole =
  | "reference"
  | "own-work"
  | "team-work"
  | "unspecified"
  | (string & {})

export type WikiExtractionStatus = "complete" | "partial" | (string & {})

export interface WikiSource {
  id: string
  source_group_id?: string | null
  source_kind?: WikiSourceKind | null
  title?: string | null
  source_title?: string | null
  source_summary?: string | null
  eligibility?: WikiSourceEligibility | null
  captured_at?: string | null
  occurred_at?: string | null
  material_role?: WikiMaterialRole | null
  personal_role?: string | null
  original_filename?: string | null
  source_path?: string | null
  raw_path?: string | null
  raw_hash?: string | null
  extraction_status?: WikiExtractionStatus | null
  format?: string | null
  source_url?: string | null
  author?: string | null
  annotation_revision?: number | null
  page_count?: number | null
  warnings?: string[]
  project_ids?: string[] | null
  area_ids?: string[] | null
}

export interface WikiProjectBinding {
  id: string
  vault_id: string
  db_instance_id: string
  root_folder_id: number
  project_note_id?: string | null
  root_folder_name?: string | null
  root_folder_path?: string | null
  created_at?: string | null
  updated_at?: string | null
}

export type WikiMemoryPageType =
  | "turn-summary"
  | "session-summary"
  | (string & {})

export interface WikiMemoryNote {
  rel: string
  page_type: WikiMemoryPageType
  title?: string | null
  summary?: string | null
  occurred_at?: string | null
  conversation_id?: string | null
  source_id?: string | null
  project_binding_ids?: string[] | null
  codeg_note_id?: string | null
}

export interface WikiMemoryNoteGroup {
  project: WikiProjectBinding
  notes: WikiMemoryNote[]
}

export interface WikiImportFile {
  filename: string
  mime?: string | null
  bytes_base64: string
}

export interface WikiImportResult extends WikiSource {
  duplicate: boolean
  warnings?: string[]
  extraction_status?: WikiExtractionStatus | null
  page_count?: number | null
  original_filename?: string | null
  format?: string | null
}

export interface WikiImportFileResult {
  filename: string
  request_id: string
  source?: WikiSource | null
  duplicate: boolean
  status: "succeeded" | "duplicate" | "failed"
  job_id?: string | null
  job_status?: string | null
  error?: string | null
}

export interface WikiImportBatchResult {
  request_id: string
  batch_id?: string | null
  results: WikiImportFileResult[]
  succeeded: number
  failed: number
  duplicates: number
}

export interface WikiBulkImportResult {
  imported: number
  duplicates: number
  failed: number
  skipped: number
  compile_job_id?: string | null
  errors?: string[]
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

function asModelPrompt(
  raw: Partial<WikiModelPromptSettings> | null | undefined
): WikiModelPromptSettings {
  return {
    model_id: emptyToNull(raw?.model_id),
    prompt: emptyToNull(raw?.prompt),
  }
}

export function wikiSynthesizeEnabled(
  raw: WikiSettingsIncoming | null | undefined
): boolean {
  if (raw?.synthesize?.enabled != null) return raw.synthesize.enabled !== false
  if (raw?.compile?.enabled != null) return raw.compile.enabled !== false
  return true
}

export function normalizeWikiSettings(
  raw: WikiSettingsIncoming | null | undefined,
  fallbackTimezone = systemTimeZone()
): WikiSettingsView {
  const capture = raw?.capture
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
    turn_summary: asModelPrompt(raw?.turn_summary),
    session_rollup: asModelPrompt(raw?.session_rollup),
    synthesize: {
      enabled: wikiSynthesizeEnabled(raw),
      model_id: emptyToNull(raw?.synthesize?.model_id),
      prompt: emptyToNull(raw?.synthesize?.prompt),
    },
    next_compile_at: raw?.next_compile_at ?? null,
    pending_source_count:
      typeof raw?.pending_source_count === "number"
        ? raw.pending_source_count
        : undefined,
    turn_summary_builtin_prompt: raw?.turn_summary_builtin_prompt ?? "",
    session_rollup_builtin_prompt: raw?.session_rollup_builtin_prompt ?? "",
    synthesize_builtin_prompt: raw?.synthesize_builtin_prompt ?? "",
  }
}

export function effectiveWikiPrompt(
  stored: string | null | undefined,
  builtin: string | null | undefined
): string {
  return emptyToNull(stored) ?? builtin ?? ""
}

function promptOrNullIfBuiltin(
  stored: string | null | undefined,
  builtin: string | null | undefined
): string | null {
  const value = emptyToNull(stored)
  if (!value) return null
  const built = builtin?.trim() ?? ""
  if (built && value === built) return null
  return value
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
    turn_summary: {
      model_id: emptyToNull(view.turn_summary.model_id),
      prompt: promptOrNullIfBuiltin(
        view.turn_summary.prompt,
        view.turn_summary_builtin_prompt
      ),
    },
    session_rollup: {
      model_id: emptyToNull(view.session_rollup.model_id),
      prompt: promptOrNullIfBuiltin(
        view.session_rollup.prompt,
        view.session_rollup_builtin_prompt
      ),
    },
    synthesize: {
      enabled: view.synthesize.enabled,
      model_id: emptyToNull(view.synthesize.model_id),
      prompt: promptOrNullIfBuiltin(
        view.synthesize.prompt,
        view.synthesize_builtin_prompt
      ),
    },
  }
}

export function wikiProjectDisplayTitle(project: WikiProjectBinding): string {
  return (
    emptyToNull(project.root_folder_name) ??
    emptyToNull(project.root_folder_path)
      ?.split(/[/\\]/)
      .filter(Boolean)
      .pop() ??
    `Project ${project.root_folder_id}`
  )
}

export function wikiJobKindKey(
  kind: string | null | undefined
): WikiJobKindKey {
  switch (kind) {
    case "turn_summary":
    case "session_rollup":
    case "wiki_synthesize":
    case "ingest":
    case "compile":
      return kind
    default:
      return "unknown"
  }
}

export function wikiJobErrorMessage(
  job: WikiJob | null | undefined
): string | null {
  if (!job) return null
  return (
    emptyToNull(job.error_message) ??
    emptyToNull(job.error) ??
    emptyToNull(job.message)
  )
}

function jobTimestampMs(job: WikiJob): number {
  for (const value of [
    job.finished_at,
    job.updated_at,
    job.started_at,
    job.created_at,
  ]) {
    const trimmed = emptyToNull(value)
    if (!trimmed) continue
    const ms = Date.parse(trimmed)
    if (!Number.isNaN(ms)) return ms
  }
  return 0
}

/** Latest failed organize job. Ignores retired `compile` / `ingest` failures. */
export function latestFailedWikiSynthesizeJob(jobs: WikiJob[]): WikiJob | null {
  const failed = jobs.filter(
    (job) =>
      job.kind === "wiki_synthesize" &&
      (job.status == null || job.status === "failed")
  )
  if (failed.length === 0) return null
  return failed.reduce((latest, job) =>
    jobTimestampMs(job) >= jobTimestampMs(latest) ? job : latest
  )
}

export function isWikiMemoryNote(
  note: Pick<WikiMemoryNote, "page_type">
): boolean {
  const type = note.page_type
  return (
    type === "turn-summary" ||
    type === "session-summary" ||
    type === "turn_summary" ||
    type === "session_rollup"
  )
}

export function wikiMemoryPageTypeKey(
  pageType: string | null | undefined
): "turn-summary" | "session-summary" | "unknown" {
  if (pageType === "turn-summary" || pageType === "turn_summary") {
    return "turn-summary"
  }
  if (pageType === "session-summary" || pageType === "session_rollup") {
    return "session-summary"
  }
  return "unknown"
}

export function wikiMemoryNoteTitle(note: WikiMemoryNote): string {
  return (
    emptyToNull(note.title) ?? emptyToNull(note.rel) ?? note.source_id ?? ""
  )
}

export function wikiMemoryNoteKey(note: WikiMemoryNote): string {
  return (
    emptyToNull(note.rel) ??
    emptyToNull(note.codeg_note_id) ??
    emptyToNull(note.source_id) ??
    `${note.page_type}:${note.conversation_id ?? ""}`
  )
}

function compareMemoryNotes(a: WikiMemoryNote, b: WikiMemoryNote): number {
  const ta = a.occurred_at ? Date.parse(a.occurred_at) : Number.NaN
  const tb = b.occurred_at ? Date.parse(b.occurred_at) : Number.NaN
  const aOk = !Number.isNaN(ta)
  const bOk = !Number.isNaN(tb)
  if (aOk && bOk && tb !== ta) return tb - ta
  if (aOk !== bOk) return aOk ? -1 : 1
  return wikiMemoryNoteTitle(a).localeCompare(wikiMemoryNoteTitle(b))
}

export function groupWikiMemoryNotesByProject(
  notes: WikiMemoryNote[],
  projects: WikiProjectBinding[]
): { groups: WikiMemoryNoteGroup[]; ungrouped: WikiMemoryNote[] } {
  const memory = notes.filter(isWikiMemoryNote)
  const known = new Map(projects.map((project) => [project.id, project]))
  const byProject = new Map<string, WikiMemoryNote[]>()
  const ungrouped: WikiMemoryNote[] = []

  for (const note of memory) {
    const ids = (note.project_binding_ids ?? []).filter(
      (id) => typeof id === "string" && id.length > 0 && known.has(id)
    )
    if (ids.length === 0) {
      ungrouped.push(note)
      continue
    }
    const seen = new Set<string>()
    for (const id of ids) {
      if (seen.has(id)) continue
      seen.add(id)
      const list = byProject.get(id) ?? []
      list.push(note)
      byProject.set(id, list)
    }
  }

  const groups: WikiMemoryNoteGroup[] = []
  for (const project of projects) {
    const list = byProject.get(project.id)
    if (!list?.length) continue
    groups.push({ project, notes: [...list].sort(compareMemoryNotes) })
  }

  return {
    groups,
    ungrouped: [...ungrouped].sort(compareMemoryNotes),
  }
}

export function wikiSourceCardTitle(source: WikiSource): string {
  return (
    wikiSourceTitle(source) ?? emptyToNull(source.source_summary) ?? source.id
  )
}

export function normalizeWikiList<T>(payload: unknown): WikiListPage<T> {
  if (Array.isArray(payload)) {
    return { items: payload as T[], total: payload.length }
  }
  if (payload && typeof payload === "object") {
    const obj = payload as Record<string, unknown>
    for (const key of [
      "items",
      "notes",
      "jobs",
      "sources",
      "rows",
      "entries",
    ]) {
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

const HIDDEN_VAULT_NAMES = new Set([".obsidian", ".git"])

export function filterWikiVaultNoise(
  nodes: WikiVaultTreeNode[],
  options?: { includeRaw?: boolean }
): WikiVaultTreeNode[] {
  const includeRaw = options?.includeRaw === true
  const out: WikiVaultTreeNode[] = []
  for (const node of nodes) {
    const name = wikiNodeName(node)
    if (HIDDEN_VAULT_NAMES.has(name.toLowerCase())) continue
    if (!includeRaw && name.toLowerCase() === "raw") continue
    const children = node.children
      ? filterWikiVaultNoise(node.children, options)
      : node.children
    out.push(children === node.children ? node : { ...node, children })
  }
  return out
}

export function vaultNodesUnderPrefix(
  nodes: WikiVaultTreeNode[],
  prefix: string
): WikiVaultTreeNode[] {
  const p = prefix.replace(/\\/g, "/").replace(/\/+$/, "")
  if (!p) return nodes

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

export type WikiMarkdownPart =
  | { kind: "text"; text: string }
  | {
      kind: "wikilink"
      raw: string
      label: string
      target: string | null
    }

/** Vault-relative note path without `.md`. Null when the link leaves the vault. */
export function sanitizeWikilinkPath(raw: string): string | null {
  let p = raw.trim().replace(/\\/g, "/")
  const hash = p.indexOf("#")
  if (hash >= 0) p = p.slice(0, hash).trim()
  if (!p) return null
  const lower = p.toLowerCase()
  if (lower.startsWith("http://") || lower.startsWith("https://")) {
    return null
  }
  if (/^[a-z][a-z0-9+.-]*:/i.test(p)) return null
  if (p.startsWith("/")) return null
  const parts: string[] = []
  for (const seg of p.split("/")) {
    if (!seg || seg === ".") continue
    if (seg === "..") return null
    parts.push(seg)
  }
  if (parts.length === 0) return null
  let joined = parts.join("/")
  if (joined.toLowerCase().endsWith(".md")) {
    joined = joined.slice(0, -3)
  }
  return joined
}

export function parseWikilink(inner: string): {
  target: string | null
  label: string
} {
  const pipe = inner.indexOf("|")
  const pathPart = (pipe >= 0 ? inner.slice(0, pipe) : inner).trim()
  const label =
    (pipe >= 0 ? inner.slice(pipe + 1) : pathPart).trim() || pathPart
  return { target: sanitizeWikilinkPath(pathPart), label }
}

export function resolveWikiNotePath(target: string): string | null {
  const sanitized = sanitizeWikilinkPath(target)
  if (!sanitized) return null
  return `${sanitized}.md`
}

export function splitWikiMarkdown(content: string): WikiMarkdownPart[] {
  if (!content) return []
  const parts: WikiMarkdownPart[] = []
  const re = /\[\[([^\]\n]+?)\]\]/g
  let last = 0
  let match: RegExpExecArray | null
  while ((match = re.exec(content)) !== null) {
    if (match.index > last) {
      parts.push({
        kind: "text",
        text: content.slice(last, match.index),
      })
    }
    const parsed = parseWikilink(match[1] ?? "")
    parts.push({
      kind: "wikilink",
      raw: match[0],
      label: parsed.label,
      target: parsed.target,
    })
    last = match.index + match[0].length
  }
  if (last < content.length) {
    parts.push({ kind: "text", text: content.slice(last) })
  }
  return parts
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
