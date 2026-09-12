import { readFileSync } from "node:fs"
import { resolve } from "node:path"

import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import { NextIntlClientProvider } from "next-intl"
import { beforeEach, describe, expect, it, vi } from "vitest"

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: vi.fn() }),
}))

vi.mock("@/lib/api", () => ({
  acpListAgents: vi.fn(),
  listModelProviders: vi.fn(),
  acpPreflight: vi.fn(),
  acpUpdateAgentEnv: vi.fn(),
}))

vi.mock("sonner", () => ({
  toast: {
    success: vi.fn(),
    error: vi.fn(),
    info: vi.fn(),
  },
}))

import enMessages from "@/i18n/messages/en.json"
import {
  acpListAgents,
  acpPreflight,
  acpUpdateAgentEnv,
  listModelProviders,
} from "@/lib/api"
import type { AcpAgentInfo, ModelProviderInfo } from "@/lib/types"
import { CodegAgentSettings } from "./codeg-agent-settings"

const mockListAgents = vi.mocked(acpListAgents)
const mockListProviders = vi.mocked(listModelProviders)
const mockPreflight = vi.mocked(acpPreflight)
const mockUpdateEnv = vi.mocked(acpUpdateAgentEnv)

function agent(overrides: Partial<AcpAgentInfo> = {}): AcpAgentInfo {
  return {
    agent_type: "codeg_agent",
    skills_capable: true,
    registry_id: "codeg_agent",
    registry_version: "0.30.6",
    supports_custom_version: false,
    name: "Codeg Agent",
    description: "",
    available: true,
    distribution_type: "in_process",
    is_acp_adapter: false,
    custom_source: null,
    enabled: false,
    sort_order: 0,
    installed_version: "0.30.6",
    host_tools_agent_mode: false,
    env: {
      CODEG_AGENT_COMPACT_SOFT_PERCENT: "70",
      CODEG_AGENT_MAX_TURNS: "24",
    },
    config_json: null,
    config_file_path: null,
    opencode_auth_json: null,
    codex_auth_json: null,
    codex_config_toml: null,
    codex_model_catalog: null,
    codex_sandbox_settings: null,
    cline_secrets_json: null,
    hermes_config_yaml: null,
    grok_config_toml: null,
    grok_settings: null,
    cursor_cli_config_json: null,
    cursor_settings: null,
    model_provider_id: 1,
    icon_url: null,
    ...overrides,
  } as AcpAgentInfo
}

function provider(
  overrides: Partial<ModelProviderInfo> = {}
): ModelProviderInfo {
  return {
    id: 1,
    name: "Ollama local",
    api_url: "http://127.0.0.1:11434/v1",
    api_key: "",
    api_key_masked: "",
    agent_type: "codeg_agent",
    model: "llama3.2",
    created_at: "",
    updated_at: "",
    ...overrides,
  }
}

function renderPage() {
  return render(
    <NextIntlClientProvider locale="en" messages={enMessages}>
      <CodegAgentSettings />
    </NextIntlClientProvider>
  )
}

describe("Codeg Agent dedicated settings", () => {
  const page = readFileSync(
    resolve(process.cwd(), "src/components/settings/codeg-agent-settings.tsx"),
    "utf8"
  )
  const shell = readFileSync(
    resolve(process.cwd(), "src/components/settings/settings-shell.tsx"),
    "utf8"
  )
  const dialog = readFileSync(
    resolve(
      process.cwd(),
      "src/components/settings/add-model-provider-dialog.tsx"
    ),
    "utf8"
  )

  it("exposes four setting groups and reuses model-provider bind", () => {
    expect(page).toContain('title={t("providerTitle")}')
    expect(page).toContain('title={t("promptsTitle")}')
    expect(page).toContain('title={t("compressionTitle")}')
    expect(page).toContain('title={t("runtimeTitle")}')
    expect(page).toContain("bindCodegProviderEnv")
    expect(page).toContain('defaultAgentType="codeg_agent"')
    expect(page).toContain("overlayCodegPromptEnv")
    expect(page).toContain("persistThenRunPreflight")
    expect(page).toContain("CODEG_COMPACT_SOFT_PERCENT_KEY")
    expect(page).toContain("CODEG_MAX_TURNS_KEY")
  })

  it("adds a settings nav item for the built-in agent", () => {
    expect(shell).toContain('href: "/settings/codeg-agent"')
    expect(shell).toContain('labelKey: "codeg_agent"')
  })

  it("lets Codeg Completions presets omit an API key on loopback", () => {
    expect(dialog).toContain("CODEG_PROVIDER_PRESETS")
    expect(dialog).toContain("isLoopbackHttpUrl")
    expect(dialog).toContain("defaultAgentType")
  })

  beforeEach(() => {
    mockListAgents.mockReset()
    mockListProviders.mockReset()
    mockPreflight.mockReset()
    mockUpdateEnv.mockReset()
    mockListAgents.mockResolvedValue([agent()])
    mockListProviders.mockResolvedValue([provider()])
    mockPreflight.mockResolvedValue({
      agent_type: "codeg_agent",
      agent_name: "Codeg Agent",
      passed: true,
      checks: [],
      adapter: null,
    })
    mockUpdateEnv.mockResolvedValue(0)
  })

  it("renders provider, prompt, compression, and runtime cards", async () => {
    renderPage()
    await waitFor(() => {
      expect(
        screen.getByRole("heading", { name: "Built-in agent" })
      ).toBeInTheDocument()
    })
    expect(
      screen.getByRole("heading", { name: "Model provider" })
    ).toBeInTheDocument()
    expect(screen.getByRole("heading", { name: "Prompts" })).toBeInTheDocument()
    expect(
      screen.getByRole("heading", { name: "Context compression" })
    ).toBeInTheDocument()
    expect(screen.getByRole("heading", { name: "Runtime" })).toBeInTheDocument()
    expect(screen.getByDisplayValue("70")).toBeInTheDocument()
    expect(screen.getByDisplayValue("24")).toBeInTheDocument()
    expect(
      screen.getByRole("switch", { name: "Enable Codeg Agent" })
    ).not.toBeChecked()
  })

  it("saves bind, prompts, compression, and runtime through agent env", async () => {
    renderPage()
    await waitFor(() => {
      expect(
        screen.getByRole("heading", { name: "Built-in agent" })
      ).toBeInTheDocument()
    })
    fireEvent.click(screen.getByRole("switch", { name: "Enable Codeg Agent" }))
    fireEvent.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() => {
      expect(mockUpdateEnv).toHaveBeenCalled()
    })
    const payload = mockUpdateEnv.mock.calls[0]?.[1]
    expect(payload?.enabled).toBe(true)
    expect(payload?.modelProviderId).toBe(1)
    expect(payload?.env.CODEG_AGENT_COMPACT_SOFT_PERCENT).toBe("70")
    expect(payload?.env.CODEG_AGENT_MAX_TURNS).toBe("24")
    expect(mockPreflight).toHaveBeenCalled()
  })
})
