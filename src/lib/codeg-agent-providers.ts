import { getAgentLabel } from "@/lib/custom-agents"
import {
  MODEL_PROVIDER_AGENT_TYPES,
  type AgentType,
  type ModelProviderInfo,
} from "@/lib/types"

/**
 * OpenAI Chat Completions `/v1` presets for Codeg Agent. The runtime is Rig's
 * CompletionsClient; cloud, local, and custom gateways all bind the same way.
 */
export const CODEG_PROVIDER_PRESET_IDS = [
  "openai",
  "deepseek",
  "groq",
  "together",
  "openrouter",
  "ollama",
  "lmstudio",
  "custom",
] as const

export type CodegProviderPresetId = (typeof CODEG_PROVIDER_PRESET_IDS)[number]

export interface CodegProviderPreset {
  id: CodegProviderPresetId
  apiUrl: string
  model: string
  local: boolean
}

export const CODEG_PROVIDER_PRESETS: readonly CodegProviderPreset[] = [
  {
    id: "openai",
    apiUrl: "https://api.openai.com/v1",
    model: "gpt-4.1",
    local: false,
  },
  {
    id: "deepseek",
    apiUrl: "https://api.deepseek.com/v1",
    model: "deepseek-chat",
    local: false,
  },
  {
    id: "groq",
    apiUrl: "https://api.groq.com/openai/v1",
    model: "llama-3.3-70b-versatile",
    local: false,
  },
  {
    id: "together",
    apiUrl: "https://api.together.xyz/v1",
    model: "meta-llama/Llama-3.3-70B-Instruct-Turbo",
    local: false,
  },
  {
    id: "openrouter",
    apiUrl: "https://openrouter.ai/api/v1",
    model: "openai/gpt-4.1",
    local: false,
  },
  {
    id: "ollama",
    apiUrl: "http://127.0.0.1:11434/v1",
    model: "llama3.2",
    local: true,
  },
  {
    id: "lmstudio",
    apiUrl: "http://127.0.0.1:1234/v1",
    model: "",
    local: true,
  },
  { id: "custom", apiUrl: "", model: "", local: false },
]

export function codegProviderPreset(
  id: string
): CodegProviderPreset | undefined {
  return CODEG_PROVIDER_PRESETS.find((preset) => preset.id === id)
}

function httpHost(raw: string): string | null {
  const trimmed = raw.trim()
  const rest = trimmed.startsWith("https://")
    ? trimmed.slice("https://".length)
    : trimmed.startsWith("http://")
      ? trimmed.slice("http://".length)
      : null
  if (rest == null) return null
  const hostport = rest.split("/")[0] ?? ""
  const at = hostport.lastIndexOf("@")
  const host = at >= 0 ? hostport.slice(at + 1) : hostport
  if (host.startsWith("[")) {
    const end = host.indexOf("]")
    return (end >= 0 ? host.slice(1, end) : host.slice(1)).toLowerCase()
  }
  return host.split(":")[0]?.toLowerCase() ?? null
}

/** Local OpenAI-compatible servers (Ollama, LM Studio) typically ignore auth. */
export function isLoopbackHttpUrl(raw: string): boolean {
  const host = httpHost(raw)
  return (
    host === "localhost" ||
    host === "127.0.0.1" ||
    host === "::1" ||
    host === "0.0.0.0"
  )
}

export function modelProvidersForAgent(
  agentType: AgentType,
  providers: ModelProviderInfo[]
): ModelProviderInfo[] {
  if (agentType === "codeg_agent") {
    const allowed = new Set<string>(MODEL_PROVIDER_AGENT_TYPES)
    return providers.filter((provider) => allowed.has(provider.agent_type))
  }
  return providers.filter((provider) => provider.agent_type === agentType)
}

export function modelProviderOptionLabel(
  provider: ModelProviderInfo,
  forAgent: AgentType
): string {
  if (forAgent === "codeg_agent" && provider.agent_type !== "codeg_agent") {
    return `${provider.name} · ${getAgentLabel(provider.agent_type)}`
  }
  return provider.name
}
