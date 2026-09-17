/**
 * 个人 Wiki 的前后端数据契约，以及设置表单和展示标题使用的纯转换函数。
 * 与 Rust 的设置、资料、任务及读取模型对应；不发请求，也不在前端推断来源或仓库事实。
 */

export const WIKI_DEFAULT_COMPILE_CRON = "0 3 * * *"

export interface WikiCaptureSettings {
  acp_enabled: boolean
  exclude_agent_types: string[]
  exclude_folder_ids: number[]
}

export interface WikiModelPromptSettings {
  provider_id: number | null
  model_id: string | null
  prompt: string | null
}

export interface WikiSynthesizeSettings extends WikiModelPromptSettings {
  enabled: boolean
}

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
  turn_summary_builtin_task?: string
  session_rollup_builtin_task?: string
  synthesize_builtin_task?: string
  resolved_vault_path?: string | null
}

/** Current Wiki settings. Retired settings are intentionally not migrated. */
export type WikiSettingsIncoming = Omit<
  Partial<WikiSettingsView>,
  "turn_summary" | "session_rollup" | "synthesize"
> & {
  turn_summary?: Partial<WikiModelPromptSettings> | null
  session_rollup?: Partial<WikiModelPromptSettings> | null
  synthesize?: Partial<WikiSynthesizeSettings> | null
}

export type WikiJobKind =
  | "turn_summary"
  | "session_rollup"
  | "wiki_synthesize"
  | (string & {})

export type WikiJobKindKey =
  | "turn_summary"
  | "session_rollup"
  | "wiki_synthesize"
  | "unknown"

export type WikiJobStatus =
  | "queued"
  | "running"
  | "succeeded"
  | "failed"
  | "cancelled"
  | (string & {})

export interface WikiJob {
  vault_id?: string
  source_id?: string | null
  title?: string | null
  input_manifest?: string | null
  output_manifest?: string | null
  output_availability?: Record<string, "available" | "missing" | "conflict">
  attempts?: {
    attempt: number
    status: string
    started_at: string | null
    finished_at: string | null
    error_code: string | null
    error_message: string | null
    input_manifest: string | null
    output_manifest: string | null
  }[]
  result?: WikiJobResult | null
  next_attempt_at?: string | null
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
  error?: string | null
}

export interface WikiImportBatchResult {
  /** Partially extracted items included in succeeded. */
  partial?: number
  request_id: string
  batch_id?: string | null
  results: WikiImportFileResult[]
  succeeded: number
  failed: number
  duplicates: number
}

export interface WikiBulkImportResult {
  /** Partially extracted items included in imported. */
  partial?: number
  results?: WikiImportFileResult[]
  imported: number
  duplicates: number
  failed: number
  skipped: number
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
  name: string
  title?: string | null
  is_dir: boolean
  children?: WikiVaultTreeNode[]
}

export interface WikiVaultFile {
  path: string
  content: string
}

export interface WikiLibrary {
  vault_path: string
  home_path: string
  tree: WikiVaultTreeNode[]
  warnings: string[]
}

/** 设置页只转换常规每日时间，用户的自定义 cron 保持原样。 */
export function dailyCronTime(cron: string): string | null {
  const match = /^(\d{1,2}) (\d{1,2}) \* \* \*$/.exec(cron.trim())
  if (!match || +match[1] > 59 || +match[2] > 23) return null
  return `${match[2].padStart(2, "0")}:${match[1].padStart(2, "0")}`
}
export function cronForDailyTime(time: string): string | null {
  const match = /^(\d{2}):(\d{2})$/.exec(time)
  return match && +match[1] < 24 && +match[2] < 60
    ? `${+match[2]} ${+match[1]} * * *`
    : null
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
    provider_id:
      typeof raw?.provider_id === "number" && Number.isInteger(raw.provider_id)
        ? raw.provider_id
        : null,
    model_id: emptyToNull(raw?.model_id),
    prompt: emptyToNull(raw?.prompt),
  }
}

export function wikiSynthesizeEnabled(
  raw: WikiSettingsIncoming | null | undefined
): boolean {
  if (raw?.synthesize?.enabled != null) return raw.synthesize.enabled !== false
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
      provider_id: asModelPrompt(raw?.synthesize).provider_id,
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
    turn_summary_builtin_task: raw?.turn_summary_builtin_task ?? "",
    session_rollup_builtin_task: raw?.session_rollup_builtin_task ?? "",
    synthesize_builtin_task: raw?.synthesize_builtin_task ?? "",
    resolved_vault_path: emptyToNull(raw?.resolved_vault_path) ?? "",
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
      provider_id: view.turn_summary.provider_id,
      model_id: emptyToNull(view.turn_summary.model_id),
      prompt: promptOrNullIfBuiltin(
        view.turn_summary.prompt,
        view.turn_summary_builtin_prompt
      ),
    },
    session_rollup: {
      provider_id: view.session_rollup.provider_id,
      model_id: emptyToNull(view.session_rollup.model_id),
      prompt: promptOrNullIfBuiltin(
        view.session_rollup.prompt,
        view.session_rollup_builtin_prompt
      ),
    },
    synthesize: {
      enabled: view.synthesize.enabled,
      provider_id: view.synthesize.provider_id,
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

export function wikiSourceTitle(source: WikiSource): string | null {
  return (
    emptyToNull(source.source_title) ??
    emptyToNull(source.title) ??
    emptyToNull(source.original_filename)
  )
}

export interface WikiNoteSummary {
  note_id: string
  path: string
  title: string
  summary: string
  type: string
  updated_at: string | null
  project_ids: string[]
  source_ids: string[]
  evidence_level: string | null
  excerpt?: string | null
  source_id?: string | null
}
export interface WikiSourceReference {
  source_id: string | null
  availability?: "available" | "missing"
  source_url?: string | null
  title: string
  path: string
  start_line: number | null
  end_line: number | null
  excerpt: string | null
}
export interface WikiNoteDocument {
  note: WikiNoteSummary
  body: string
  source: string
  format_warning: boolean
  sources: WikiSourceReference[]
  headings: { id: string; title: string; level: number }[]
}
export interface WikiSourceDocument {
  read_error?: string | null
  source: WikiSource
  body: string
  raw: string
  format_warning: boolean
  related_notes: WikiNoteSummary[]
}
export interface WikiOverview {
  enabled: boolean
  note_count: number
  source_count: number
  active_job_count: number
  pending_memory_count: number
  failed_job_count: number
  recent_notes: WikiNoteSummary[]
  next_compile_at: string | null
}
export interface WikiJobResult {
  version: number
  outcome: "generated" | "no_content" | "no_new_input" | "partial"
  reason_code: string | null
  outputs: {
    note_id: string
    path: string
    title: string
    type: string
    content_hash: string
    availability?: "available" | "missing" | "conflict"
  }[]
  processed_inputs: {
    rel: string
    content_hash: string
    disposition: string
    source_ids: string[]
  }[]
  remaining_inputs: string[]
  warnings: string[]
}
