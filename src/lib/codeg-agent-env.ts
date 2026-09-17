import {
  catalogFromProviderModel,
  codegWindowsFromCatalog,
  serializeCodegAgentCatalog,
  type CodegAgentCatalog,
  type CodegRequestProtocol,
} from "@/lib/codeg-agent-catalog"
import { isLoopbackHttpUrl } from "@/lib/codeg-agent-providers"
import {
  CODEG_BUILTIN_COMPACT_PROMPT,
  CODEG_BUILTIN_SYSTEM_PROMPT,
} from "@/lib/codeg-agent-prompts"
import { parseEnvText, patchEnvText } from "@/lib/env-text"
import {
  completionsModelIdFromProvider,
  DEFAULT_CODEG_CONTEXT_WINDOW,
  suggestedCodegContextWindow,
} from "@/lib/types"

export const CODEG_DEFAULT_MAX_OUTPUT = "4096"
export const CODEG_SYSTEM_PROMPT_KEY = "CODEG_AGENT_SYSTEM_PROMPT"
export const CODEG_COMPACT_PROMPT_KEY = "CODEG_AGENT_COMPACT_PROMPT"
export const CODEG_BIND_SELECT_ID = "codeg-agent-bind-provider"
export const CODEG_WINDOW_INPUT_ID = "codeg-agent-context-window"
export const CODEG_COMPACT_SOFT_PERCENT_KEY = "CODEG_AGENT_COMPACT_SOFT_PERCENT"
export const CODEG_COMPACT_RECENT_TURNS_KEY = "CODEG_AGENT_COMPACT_RECENT_TURNS"
export const CODEG_COMPACT_MODEL_KEY = "CODEG_AGENT_COMPACT_MODEL"
export const CODEG_PROTOCOL_KEY = "CODEG_AGENT_PROTOCOL"
export const CODEG_MAX_TURNS_KEY = "CODEG_AGENT_MAX_TURNS"
export const CODEG_DEFAULT_COMPACT_SOFT_PERCENT = "80"
export const CODEG_DEFAULT_COMPACT_RECENT_TURNS = "6"
export const CODEG_DEFAULT_MAX_TURNS = "40"
export const CODEG_INJECT_AGENTS_MD_KEY = "CODEG_AGENT_INJECT_AGENTS_MD"
export const CODEG_INJECT_CLAUDE_MD_KEY = "CODEG_AGENT_INJECT_CLAUDE_MD"
export const CODEG_INJECT_TREE_KEY = "CODEG_AGENT_INJECT_TREE"

/**
 * Parse `CODEG_AGENT_CONTEXT_WINDOWS` JSON. Invalid JSON yields `{}` so the
 * card can rewrite a single model id without inventing a 128k default here.
 */
export function parseCodegContextWindows(
  envText: string
): Record<string, number> {
  const rawWindows = parseEnvText(envText).CODEG_AGENT_CONTEXT_WINDOWS?.trim()
  if (!rawWindows) return {}
  try {
    const value = JSON.parse(rawWindows) as unknown
    if (!value || typeof value !== "object" || Array.isArray(value)) return {}
    const windows: Record<string, number> = {}
    for (const [key, raw] of Object.entries(value as Record<string, unknown>)) {
      const name = key.trim()
      const window =
        typeof raw === "number"
          ? raw
          : typeof raw === "string"
            ? Number.parseInt(raw.trim(), 10)
            : Number.NaN
      if (name && Number.isFinite(window) && window > 0) {
        windows[name] = window
      }
    }
    return windows
  } catch {
    return {}
  }
}

export function codegWindowForModel(
  envText: string,
  modelId: string
): number | null {
  const id = modelId.trim()
  if (!id) return null
  return parseCodegContextWindows(envText)[id] ?? null
}

export function patchCodegContextWindow(
  envText: string,
  modelId: string,
  windowTokens: number
): string {
  const id = modelId.trim()
  if (!id || !Number.isFinite(windowTokens) || windowTokens <= 0) return envText
  const windows = parseCodegContextWindows(envText)
  windows[id] = windowTokens
  return patchEnvText(envText, {
    CODEG_AGENT_CONTEXT_WINDOWS: JSON.stringify(windows),
  })
}

export function codegMaxOutputTokens(envText: string): string {
  return (
    parseEnvText(envText).CODEG_AGENT_MAX_OUTPUT_TOKENS?.trim() ||
    CODEG_DEFAULT_MAX_OUTPUT
  )
}

export function patchCodegMaxOutputTokens(
  envText: string,
  value: string
): string {
  const trimmed = value.trim()
  return patchEnvText(envText, {
    CODEG_AGENT_MAX_OUTPUT_TOKENS: trimmed || CODEG_DEFAULT_MAX_OUTPUT,
  })
}

export function codegEnvInt(
  envText: string,
  key: string,
  fallback: string
): string {
  return parseEnvText(envText)[key]?.trim() || fallback
}

export function patchCodegEnvInt(
  envText: string,
  key: string,
  value: string,
  fallback: string
): string {
  const trimmed = value.trim()
  return patchEnvText(envText, {
    [key]: trimmed || fallback,
  })
}

/** Matches Rust `parse_flag`: 1 / true / yes / on. Missing or other values are off. */
export function codegFlag(envText: string, key: string): boolean {
  const raw = parseEnvText(envText)[key]?.trim().toLowerCase()
  return raw === "1" || raw === "true" || raw === "yes" || raw === "on"
}

/** On writes `1`; off deletes the key so spawn keeps the default (off). */
export function patchCodegFlag(
  envText: string,
  key: string,
  on: boolean
): string {
  return patchEnvText(envText, { [key]: on ? "1" : "" })
}

function promptOverrideOrDelete(value: string, builtin: string): string | null {
  const trimmed = value.trim()
  if (!trimmed || trimmed === builtin.trim()) return null
  return trimmed
}

/**
 * Overlay multiline prompts onto a parsed env map. Empty trim, or text equal
 * to the built-in default, deletes the key so spawn uses the Rust constant.
 * The KEY=VALUE textarea never holds these.
 */
export function overlayCodegPromptEnv(
  env: Record<string, string>,
  systemPrompt: string,
  compactPrompt: string
): Record<string, string> {
  const next = { ...env }
  const system = promptOverrideOrDelete(
    systemPrompt,
    CODEG_BUILTIN_SYSTEM_PROMPT
  )
  const compact = promptOverrideOrDelete(
    compactPrompt,
    CODEG_BUILTIN_COMPACT_PROMPT
  )
  if (system) next[CODEG_SYSTEM_PROMPT_KEY] = system
  else delete next[CODEG_SYSTEM_PROMPT_KEY]
  if (compact) next[CODEG_COMPACT_PROMPT_KEY] = compact
  else delete next[CODEG_COMPACT_PROMPT_KEY]
  return next
}

export function codegCompactModel(envText: string): string {
  return parseEnvText(envText)[CODEG_COMPACT_MODEL_KEY]?.trim() ?? ""
}

export function patchCodegCompactModel(
  envText: string,
  modelId: string
): string {
  return patchEnvText(envText, {
    [CODEG_COMPACT_MODEL_KEY]: modelId.trim(),
  })
}

export function replaceCodegContextWindows(
  envText: string,
  windows: Record<string, number>
): string {
  const entries = Object.entries(windows).filter(
    ([id, window]) => id.trim() && Number.isFinite(window) && window > 0
  )
  if (entries.length === 0) {
    return patchEnvText(envText, { CODEG_AGENT_CONTEXT_WINDOWS: "" })
  }
  const next: Record<string, number> = {}
  for (const [id, window] of entries) {
    next[id] = window
  }
  return patchEnvText(envText, {
    CODEG_AGENT_CONTEXT_WINDOWS: JSON.stringify(next),
  })
}

export function codegWindowsFromProvider(provider: {
  agent_type: string
  model?: string | null
}): Record<string, number> {
  const catalog = catalogFromProviderModel(provider.model)
  if (catalog) return codegWindowsFromCatalog(catalog)
  const id = completionsModelIdFromProvider(provider)
  if (!id) return {}
  return { [id]: suggestedCodegContextWindow(provider) }
}

/** Save then refresh preflight. Used by the Codeg config card and enable switch. */
export async function persistThenRunPreflight(
  persist: () => Promise<unknown>,
  runPreflight: () => Promise<unknown>
): Promise<void> {
  await persist()
  await runPreflight()
}

/**
 * Ensure the bound model has a window (and a default max-output) in env text.
 * Existing entries are kept; 128000 is only written when the id is missing.
 */
export function ensureCodegLaunchEnv(
  envText: string,
  modelId: string,
  windowTokens: number
): string {
  const id = modelId.trim()
  const parsed = parseEnvText(envText)
  const windows = parseCodegContextWindows(envText)
  if (id && windows[id] == null) {
    windows[id] = windowTokens > 0 ? windowTokens : DEFAULT_CODEG_CONTEXT_WINDOW
  }
  const patch: Record<string, string | undefined> = {}
  if (Object.keys(windows).length > 0) {
    patch.CODEG_AGENT_CONTEXT_WINDOWS = JSON.stringify(windows)
  }
  if (!parsed.CODEG_AGENT_MAX_OUTPUT_TOKENS?.trim()) {
    patch.CODEG_AGENT_MAX_OUTPUT_TOKENS = CODEG_DEFAULT_MAX_OUTPUT
  }
  return patchEnvText(envText, patch)
}

export function bindCodegProviderEnv(
  envText: string,
  provider: {
    api_url: string
    api_key: string
    agent_type: string
    model?: string | null
  } | null
): { envText: string; model: string } {
  const apiUrl = provider?.api_url?.trim() ?? ""
  const apiKey = provider?.api_key?.trim() ?? ""
  const model = provider ? completionsModelIdFromProvider(provider) : ""
  const windows = provider ? codegWindowsFromProvider(provider) : {}
  const windowTokens =
    (model ? windows[model] : undefined) ??
    (provider
      ? suggestedCodegContextWindow(provider)
      : DEFAULT_CODEG_CONTEXT_WINDOW)
  const catalog = provider ? catalogFromProviderModel(provider.model) : null
  let next = patchEnvText(envText, {
    CODEG_AGENT_API_BASE_URL: apiUrl,
    CODEG_AGENT_API_KEY: apiKey,
    CODEG_AGENT_MODEL: model,
    [CODEG_PROTOCOL_KEY]: catalog?.protocol ?? "",
  })
  next = replaceCodegContextWindows(next, windows)
  next = ensureCodegLaunchEnv(next, model, windowTokens)
  const compact = codegCompactModel(next)
  if (compact && !windows[compact]) {
    next = patchCodegCompactModel(next, "")
  }
  return { model, envText: next }
}

export type CodegProtocolProbe = (params: {
  baseUrl: string
  apiKey: string
  modelId: string
}) => Promise<Exclude<CodegRequestProtocol, "auto">>

/** Bind a provider after probing Completions vs Responses. Locks the winner. */
export async function bindCodegProviderWithProbe(
  envText: string,
  provider: {
    api_url: string
    api_key: string
    agent_type: string
    model?: string | null
  } | null,
  probe: CodegProtocolProbe
): Promise<{
  envText: string
  model: string
  protocol: Exclude<CodegRequestProtocol, "auto"> | ""
  catalog: CodegAgentCatalog | null
}> {
  if (!provider) {
    const bound = bindCodegProviderEnv(envText, null)
    return { ...bound, protocol: "", catalog: null }
  }
  const model = completionsModelIdFromProvider(provider)
  const apiKey =
    provider.api_key.trim() ||
    (isLoopbackHttpUrl(provider.api_url) ? "local" : "")
  const protocol = await probe({
    baseUrl: provider.api_url.trim(),
    apiKey,
    modelId: model,
  })
  const catalog = catalogFromProviderModel(provider.model)
  const nextCatalog = catalog ? { ...catalog, protocol } : null
  let bound = bindCodegProviderEnv(envText, {
    ...provider,
    model: nextCatalog
      ? serializeCodegAgentCatalog(nextCatalog)
      : provider.model,
  })
  bound = {
    ...bound,
    envText: patchEnvText(bound.envText, {
      [CODEG_PROTOCOL_KEY]: protocol,
    }),
  }
  return { ...bound, protocol, catalog: nextCatalog }
}

export function codegDraftFromEnv(env: Record<string, string>): {
  envText: string
  systemPrompt: string
  compactPrompt: string
} {
  const systemPrompt =
    env[CODEG_SYSTEM_PROMPT_KEY]?.trim() || CODEG_BUILTIN_SYSTEM_PROMPT
  const compactPrompt =
    env[CODEG_COMPACT_PROMPT_KEY]?.trim() || CODEG_BUILTIN_COMPACT_PROMPT
  const envText = patchEnvText(
    Object.entries(env)
      .map(([key, value]) => `${key}=${value}`)
      .join("\n"),
    {
      [CODEG_SYSTEM_PROMPT_KEY]: "",
      [CODEG_COMPACT_PROMPT_KEY]: "",
    }
  )
  return { envText, systemPrompt, compactPrompt }
}
