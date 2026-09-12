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
export const CODEG_MAX_TURNS_KEY = "CODEG_AGENT_MAX_TURNS"
export const CODEG_DEFAULT_COMPACT_SOFT_PERCENT = "80"
export const CODEG_DEFAULT_COMPACT_RECENT_TURNS = "6"
export const CODEG_DEFAULT_MAX_TURNS = "40"

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

/**
 * Overlay multiline prompts onto a parsed env map. Empty trim deletes the key
 * so spawn uses the builtin default. The KEY=VALUE textarea never holds these.
 */
export function overlayCodegPromptEnv(
  env: Record<string, string>,
  systemPrompt: string,
  compactPrompt: string
): Record<string, string> {
  const next = { ...env }
  const system = systemPrompt.trim()
  const compact = compactPrompt.trim()
  if (system) next[CODEG_SYSTEM_PROMPT_KEY] = system
  else delete next[CODEG_SYSTEM_PROMPT_KEY]
  if (compact) next[CODEG_COMPACT_PROMPT_KEY] = compact
  else delete next[CODEG_COMPACT_PROMPT_KEY]
  return next
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
  const windowTokens = provider
    ? suggestedCodegContextWindow(provider)
    : DEFAULT_CODEG_CONTEXT_WINDOW
  return {
    model,
    envText: ensureCodegLaunchEnv(
      patchEnvText(envText, {
        CODEG_AGENT_API_BASE_URL: apiUrl,
        CODEG_AGENT_API_KEY: apiKey,
        CODEG_AGENT_MODEL: model,
      }),
      model,
      windowTokens
    ),
  }
}

export function codegDraftFromEnv(env: Record<string, string>): {
  envText: string
  systemPrompt: string
  compactPrompt: string
} {
  const systemPrompt = env[CODEG_SYSTEM_PROMPT_KEY] ?? ""
  const compactPrompt = env[CODEG_COMPACT_PROMPT_KEY] ?? ""
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
