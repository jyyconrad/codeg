/**
 * Wiki 专用请求入口，供设置、资料导入、任务操作和点击后的笔记探测调用。
 * 桌面与 Web 统一通过 Transport；页面持续查询和刷新由 wiki-data 管理。
 */
import { getTransport } from "./transport"
import type { SelectedSessionKey } from "./types"
import type {
  WikiBulkImportResult,
  WikiImportBatchResult,
  WikiImportFile,
  WikiImportResult,
  WikiJob,
  WikiNoteDocument,
  WikiSettings,
  WikiSettingsView,
  WikiSource,
  WikiVaultTreeNode,
} from "./wiki-types"

export const wikiReadNote = (path: string) =>
  getTransport().call<WikiNoteDocument>("wiki_read_note", { path })

export async function getWikiSettings(): Promise<WikiSettingsView> {
  return getTransport().call("get_wiki_settings")
}

export async function updateWikiSettings(
  settings: WikiSettings
): Promise<WikiSettingsView> {
  return getTransport().call("update_wiki_settings", { settings })
}

export async function wikiVaultTree(params?: {
  path?: string | null
  recursive?: boolean
  includeRaw?: boolean
}): Promise<WikiVaultTreeNode[]> {
  return getTransport().call("wiki_vault_tree", {
    path: params?.path ?? null,
    recursive: params?.recursive ?? null,
    include_raw: params?.includeRaw ?? null,
  })
}

export async function wikiImportText(params: {
  request_id: string
  text: string
  title?: string | null
  source_url?: string | null
  author?: string | null
  material_role?: string | null
  personal_role?: string | null
  project_ids?: string[] | null
  area_ids?: string[] | null
}): Promise<WikiImportResult> {
  return getTransport().call("wiki_import_text", {
    request_id: params.request_id,
    text: params.text,
    title: params.title ?? null,
    source_url: params.source_url ?? null,
    author: params.author ?? null,
    material_role: params.material_role ?? null,
    personal_role: params.personal_role ?? null,
    project_ids: params.project_ids ?? null,
    area_ids: params.area_ids ?? null,
  })
}

export async function wikiImportFiles(params: {
  request_id: string
  files: WikiImportFile[]
  material_role?: string | null
  personal_role?: string | null
  title?: string | null
  source_url?: string | null
  author?: string | null
  batch_id?: string | null
  project_ids?: string[] | null
  area_ids?: string[] | null
}): Promise<WikiImportBatchResult> {
  return getTransport().call(
    "wiki_import_files",
    {
      request_id: params.request_id,
      files: params.files,
      material_role: params.material_role ?? null,
      personal_role: params.personal_role ?? null,
      title: params.title ?? null,
      source_url: params.source_url ?? null,
      author: params.author ?? null,
      batch_id: params.batch_id ?? null,
      project_ids: params.project_ids ?? null,
      area_ids: params.area_ids ?? null,
    },
    { timeoutMs: 90_000 }
  )
}

export async function wikiAcceptExtraction(
  sourceId: string
): Promise<WikiSource> {
  return getTransport().call("wiki_accept_extraction", {
    source_id: sourceId,
  })
}

export async function wikiReextract(
  sourceId: string
): Promise<WikiImportResult> {
  return getTransport().call("wiki_reextract", { source_id: sourceId })
}

export async function wikiImportLocalSessions(params: {
  request_id: string
  selections?: SelectedSessionKey[]
  all?: boolean
}): Promise<WikiBulkImportResult> {
  return getTransport().call(
    "wiki_import_local_sessions",
    {
      request_id: params.request_id,
      selections: params.selections ?? [],
      all: params.all ?? false,
    },
    { timeoutMs: 300_000 }
  )
}

export async function wikiImportDirectory(params: {
  request_id: string
  path: string
}): Promise<WikiBulkImportResult> {
  return getTransport().call(
    "wiki_import_directory",
    {
      request_id: params.request_id,
      path: params.path,
    },
    { timeoutMs: 300_000 }
  )
}

export async function wikiCompileNow(requestId: string): Promise<WikiJob> {
  return getTransport().call("wiki_compile_now", { request_id: requestId })
}

export async function wikiRetryJob(id: string): Promise<WikiJob> {
  return getTransport().call("wiki_retry_job", { id })
}

export async function wikiCancelJob(id: string): Promise<WikiJob> {
  return getTransport().call("wiki_cancel_job", { id })
}
