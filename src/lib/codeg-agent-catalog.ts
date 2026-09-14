/**
 * Codeg Agent multi-model catalog stored in `model_provider.model`.
 *
 * Plain slugs stay valid. Once the dedicated settings page adds a second
 * model (or a per-model window), the row is upgraded to JSON with
 * `kind: "codeg_agent_catalog"`. Completions id extraction reads `default`.
 */

export const CODEG_CATALOG_KIND = "codeg_agent_catalog"
export const CODEG_CATALOG_DEFAULT_WINDOW = 128000

export const CODEG_REQUEST_PROTOCOLS = [
  "chat_completions",
  "responses",
  "auto",
] as const

export type CodegRequestProtocol = (typeof CODEG_REQUEST_PROTOCOLS)[number]

export interface CodegCatalogModel {
  id: string
  name: string
  context_window: number
}

export interface CodegAgentCatalog {
  kind: typeof CODEG_CATALOG_KIND
  version: 1
  protocol: CodegRequestProtocol
  default: string
  models: CodegCatalogModel[]
}

export function parseCodegRequestProtocol(raw: unknown): CodegRequestProtocol {
  return raw === "responses" || raw === "auto" ? raw : "chat_completions"
}

function positiveWindow(raw: unknown): number {
  const value =
    typeof raw === "number"
      ? raw
      : typeof raw === "string"
        ? Number.parseInt(raw.trim(), 10)
        : Number.NaN
  return Number.isFinite(value) && value > 0
    ? Math.floor(value)
    : CODEG_CATALOG_DEFAULT_WINDOW
}

function parseModelEntry(item: unknown): CodegCatalogModel | null {
  if (typeof item === "string") {
    const id = item.trim()
    if (!id) return null
    return {
      id,
      name: id,
      context_window: CODEG_CATALOG_DEFAULT_WINDOW,
    }
  }
  if (!item || typeof item !== "object" || Array.isArray(item)) return null
  const row = item as Record<string, unknown>
  const id = String(row.id ?? row.slug ?? "").trim()
  if (!id) return null
  const name = String(row.name ?? row.displayName ?? id).trim() || id
  return {
    id,
    name,
    context_window: positiveWindow(row.context_window ?? row.contextWindow),
  }
}

/** Parse a Codeg catalog JSON blob. Plain slugs and Claude/Codex JSON return null. */
export function parseCodegAgentCatalog(
  raw: string | null | undefined
): CodegAgentCatalog | null {
  const text = raw?.trim() ?? ""
  if (!text.startsWith("{")) return null
  try {
    const value = JSON.parse(text) as unknown
    if (!value || typeof value !== "object" || Array.isArray(value)) return null
    const obj = value as Record<string, unknown>
    if (obj.kind !== CODEG_CATALOG_KIND) return null
    if ("main" in obj || "customs" in obj) return null
    if (!Array.isArray(obj.models)) return null
    const models: CodegCatalogModel[] = []
    const seen = new Set<string>()
    for (const item of obj.models) {
      const entry = parseModelEntry(item)
      if (!entry || seen.has(entry.id)) continue
      seen.add(entry.id)
      models.push(entry)
    }
    if (models.length === 0) return null
    const requested = typeof obj.default === "string" ? obj.default.trim() : ""
    const defaultId = models.some((model) => model.id === requested)
      ? requested
      : models[0].id
    return {
      kind: CODEG_CATALOG_KIND,
      version: 1,
      protocol: parseCodegRequestProtocol(obj.protocol),
      default: defaultId,
      models,
    }
  } catch {
    return null
  }
}

export function serializeCodegAgentCatalog(catalog: CodegAgentCatalog): string {
  const defaultId = catalog.models.some((model) => model.id === catalog.default)
    ? catalog.default
    : (catalog.models[0]?.id ?? "")
  return JSON.stringify({
    kind: CODEG_CATALOG_KIND,
    version: 1,
    protocol: parseCodegRequestProtocol(catalog.protocol),
    default: defaultId,
    models: catalog.models.map((model) => ({
      id: model.id,
      name: model.name,
      context_window: model.context_window,
    })),
  })
}

/** Treat a non-JSON Completions slug as a one-model catalog for the editor. */
export function catalogFromPlainSlug(
  raw: string | null | undefined
): CodegAgentCatalog | null {
  const slug = raw?.trim() ?? ""
  if (!slug || slug.startsWith("{")) return null
  return {
    kind: CODEG_CATALOG_KIND,
    version: 1,
    protocol: "chat_completions",
    default: slug,
    models: [
      {
        id: slug,
        name: slug,
        context_window: CODEG_CATALOG_DEFAULT_WINDOW,
      },
    ],
  }
}

export function catalogFromProviderModel(
  raw: string | null | undefined
): CodegAgentCatalog | null {
  return parseCodegAgentCatalog(raw) ?? catalogFromPlainSlug(raw)
}

export function codegWindowsFromCatalog(
  catalog: CodegAgentCatalog
): Record<string, number> {
  const windows: Record<string, number> = {}
  for (const model of catalog.models) {
    windows[model.id] = model.context_window
  }
  return windows
}
