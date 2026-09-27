import { readFileSync } from "node:fs"
import { resolve } from "node:path"

import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react"
import { NextIntlClientProvider } from "next-intl"
import { beforeEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/api", () => ({
  getCodeIntelStatus: vi.fn(),
  getCodeIntelSettings: vi.fn(),
  setCodeIntelSettings: vi.fn(),
  retryCodeIntel: vi.fn(),
}))

vi.mock("@/stores/app-workspace-store", () => ({
  useAppWorkspaceStore: (sel: (s: never) => unknown) =>
    sel({ activeFolderId: null, allFolders: [] } as never),
}))

import * as api from "@/lib/api"
import {
  getCodeIntelSettings,
  getCodeIntelStatus,
  retryCodeIntel,
  setCodeIntelSettings,
  type CodeIntelConfig,
  type CodeIntelProviderId,
  type CodeIntelProviderStatus,
  type CodeIntelStatus,
} from "@/lib/api"
import ar from "@/i18n/messages/ar.json"
import de from "@/i18n/messages/de.json"
import enMessages from "@/i18n/messages/en.json"
import es from "@/i18n/messages/es.json"
import fr from "@/i18n/messages/fr.json"
import ja from "@/i18n/messages/ja.json"
import ko from "@/i18n/messages/ko.json"
import pt from "@/i18n/messages/pt.json"
import zhCN from "@/i18n/messages/zh-CN.json"
import zhTW from "@/i18n/messages/zh-TW.json"
import { CodeIntelligenceSettings } from "./code-intelligence-settings"

const mockGetStatus = vi.mocked(getCodeIntelStatus)
const mockGetSettings = vi.mocked(getCodeIntelSettings)
const mockRetry = vi.mocked(retryCodeIntel)
const mockSetSettings = vi.mocked(setCodeIntelSettings)

const DEFAULT_CONFIG: CodeIntelConfig = {
  enabled: false,
  lsp: {
    enabled: true,
    languages: ["rust", "go", "python", "typescript", "cpp"],
    max_concurrent: 2,
    auto_install: true,
  },
  codegraph: {
    enabled: true,
    auto_install: true,
    binary_path: null,
  },
  serena: {
    enabled: false,
    auto_install: true,
    command: null,
    version: "v1.7.0",
    context: "codex",
    modes: ["interactive", "editing", "planning"],
  },
}

function provider(
  id: CodeIntelProviderId,
  overrides: Partial<CodeIntelProviderStatus> = {}
): CodeIntelProviderStatus {
  return {
    id,
    configured: false,
    discovery: "unconfigured",
    install: "idle",
    runtime: "stopped",
    version: null,
    resolved_command: null,
    last_error: null,
    last_started_at: null,
    ...overrides,
  }
}

function defaultStatus(
  overrides: Partial<CodeIntelStatus> = {}
): CodeIntelStatus {
  return {
    config: structuredClone(DEFAULT_CONFIG),
    cwd: "/work/codeg",
    migration_report: null,
    providers: [
      provider("lsp", { configured: true, discovery: "unconfigured" }),
      provider("codegraph", { configured: true, discovery: "missing" }),
      provider("serena", { configured: false, discovery: "missing" }),
    ],
    lsp_languages: [
      {
        language: "rust",
        label: "Rust",
        checked: true,
        detected: true,
        provider: "unconfigured",
      },
      {
        language: "go",
        label: "Go",
        checked: true,
        detected: false,
        provider: "unconfigured",
      },
    ],
    ...overrides,
  }
}

function renderSettings() {
  return render(
    <NextIntlClientProvider locale="en" messages={enMessages}>
      <CodeIntelligenceSettings />
    </NextIntlClientProvider>
  )
}

function collectKeys(node: unknown, prefix = ""): string[] {
  if (typeof node === "string") return [prefix]
  if (node == null || typeof node !== "object") return [prefix]
  return Object.entries(node as Record<string, unknown>).flatMap(
    ([key, value]) => collectKeys(value, prefix ? `${prefix}.${key}` : key)
  )
}

const LOCALES = [
  ["en", enMessages],
  ["zh-CN", zhCN],
  ["zh-TW", zhTW],
  ["ja", ja],
  ["ko", ko],
  ["es", es],
  ["de", de],
  ["fr", fr],
  ["pt", pt],
  ["ar", ar],
] as const

describe("CodeIntelligenceSettings", () => {
  beforeEach(() => {
    mockGetStatus.mockReset()
    mockGetSettings.mockReset()
    mockSetSettings.mockReset()
    mockRetry.mockReset()
    mockGetStatus.mockResolvedValue(defaultStatus())
    mockRetry.mockResolvedValue(defaultStatus())
    mockSetSettings.mockImplementation(async (settings) => settings)
  })

  it("names the section and the settings nav Code tools", async () => {
    expect(enMessages.SettingsShell.nav.tools).toBe("Code tools")
    expect(zhCN.SettingsShell.nav.tools).toBe("代码工具")
    expect(zhCN.CodeIntelligenceSettings.sectionTitle).toBe("代码工具")
    expect(zhCN.CodeIntelligenceSettings.masterTitle).toBe("代码智能")
    expect(zhTW.SettingsShell.nav.tools).toBe("代碼工具")

    const enKeys = collectKeys(enMessages.CodeIntelligenceSettings).sort()
    for (const [locale, messages] of LOCALES) {
      expect(messages.SettingsShell.nav.tools, locale).not.toBe("Tools")
      expect(messages.SettingsShell.nav.tools.length, locale).toBeGreaterThan(0)
      expect(messages.CodeIntelligenceSettings.sectionTitle, locale).toBe(
        messages.SettingsShell.nav.tools
      )
      expect(
        collectKeys(messages.CodeIntelligenceSettings).sort(),
        locale
      ).toEqual(enKeys)
    }

    const shell = readFileSync(
      resolve(process.cwd(), "src/components/settings/settings-shell.tsx"),
      "utf8"
    )
    expect(shell).toContain('href: "/settings/code-intelligence"')
    expect(shell).toContain('labelKey: "tools"')

    const page = readFileSync(
      resolve(
        process.cwd(),
        "src/components/settings/code-intelligence-settings.tsx"
      ),
      "utf8"
    )
    expect(page).not.toContain("npm i -g")
    expect(page).not.toContain("goToDefinition")
    expect(page).not.toContain("codegraph_indexed")
    expect(page).not.toContain("auto_attach")
    expect(page).not.toContain("mcp_tools")

    const apiSource = readFileSync(
      resolve(process.cwd(), "src/lib/api.ts"),
      "utf8"
    )
    expect(apiSource).not.toContain("CustomLspServer")
    expect(apiSource).not.toContain("LspServerStatus")
    expect(apiSource).not.toContain("CodeIntelMcpToolStatus")

    renderSettings()
    expect(
      await screen.findByRole("heading", { name: "Code tools" })
    ).toBeInTheDocument()
    expect(screen.getByText("/work/codeg")).toBeInTheDocument()
    expect(screen.queryByText(/npm i -g/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/\.codegraph/)).not.toBeInTheDocument()
    expect(screen.queryByText("goToDefinition")).not.toBeInTheDocument()

    const columns = screen.getByTestId("code-tools-columns")
    expect(columns.className).toContain("grid-cols-1")
    expect(columns.className).toContain("md:grid-cols-2")
  })

  it("keeps the three tools visible when the master switch is off", async () => {
    renderSettings()
    const master = await screen.findByRole("switch", {
      name: "Code intelligence",
    })
    expect(master).toHaveAttribute("data-state", "unchecked")
    expect(master).toBeEnabled()

    expect(screen.getByRole("button", { name: /^LSP\b/ })).toBeInTheDocument()
    expect(
      screen.getByRole("button", { name: /^CodeGraph\b/ })
    ).toBeInTheDocument()
    expect(
      screen.getByRole("button", { name: /^Serena\b/ })
    ).toBeInTheDocument()

    expect(screen.getByRole("switch", { name: "LSP" })).toBeEnabled()
    expect(screen.getByRole("switch", { name: "CodeGraph" })).toBeEnabled()
    expect(screen.getByRole("switch", { name: "Serena" })).toBeEnabled()

    expect(screen.getByRole("button", { name: "Apply" })).toBeDisabled()
    expect(screen.getByRole("button", { name: "Check" })).toBeDisabled()
  })

  it("keeps the tool switches independent of each other and the master switch", async () => {
    renderSettings()
    const master = await screen.findByRole("switch", {
      name: "Code intelligence",
    })
    const lsp = screen.getByRole("switch", { name: "LSP" })
    const codegraph = screen.getByRole("switch", { name: "CodeGraph" })
    const serena = screen.getByRole("switch", { name: "Serena" })

    expect(lsp).toHaveAttribute("data-state", "checked")
    expect(codegraph).toHaveAttribute("data-state", "checked")
    expect(serena).toHaveAttribute("data-state", "unchecked")

    fireEvent.click(serena)
    expect(serena).toHaveAttribute("data-state", "checked")
    expect(lsp).toHaveAttribute("data-state", "checked")
    expect(codegraph).toHaveAttribute("data-state", "checked")
    expect(master).toHaveAttribute("data-state", "unchecked")
    expect(screen.getByRole("button", { name: "Apply" })).toBeDisabled()

    fireEvent.click(lsp)
    expect(lsp).toHaveAttribute("data-state", "unchecked")
    expect(serena).toHaveAttribute("data-state", "checked")
    expect(codegraph).toHaveAttribute("data-state", "checked")
    expect(master).toHaveAttribute("data-state", "unchecked")

    fireEvent.click(master)
    expect(master).toHaveAttribute("data-state", "checked")
    expect(lsp).toHaveAttribute("data-state", "unchecked")
    expect(serena).toHaveAttribute("data-state", "checked")
    expect(codegraph).toHaveAttribute("data-state", "checked")
    expect(screen.getByRole("button", { name: "Apply" })).toBeEnabled()
  })

  it("does not call a download or settings write on the initial render", async () => {
    renderSettings()
    await screen.findByRole("heading", { name: "Code tools" })

    expect(mockGetStatus).toHaveBeenCalledWith(null)
    expect(mockGetSettings).not.toHaveBeenCalled()
    expect(mockSetSettings).not.toHaveBeenCalled()
    for (const [name, value] of Object.entries(api)) {
      if (typeof value !== "function") continue
      if (!/install|download/i.test(name)) continue
      expect(value, name).not.toHaveBeenCalled()
    }

    fireEvent.click(screen.getByRole("switch", { name: "Code intelligence" }))
    const install = screen.getByRole("button", { name: "Apply" })
    expect(install).toBeEnabled()
    fireEvent.click(install)
    await waitFor(() => expect(mockSetSettings).toHaveBeenCalledTimes(1))
    expect(mockSetSettings.mock.calls[0]?.[0].enabled).toBe(true)
    for (const [name, value] of Object.entries(api)) {
      if (typeof value !== "function") continue
      if (!/install|download/i.test(name)) continue
      expect(value, name).not.toHaveBeenCalled()
    }
  })

  it("refreshes status from Check and does not download", async () => {
    renderSettings()
    await screen.findByRole("heading", { name: "Code tools" })
    fireEvent.click(screen.getByRole("switch", { name: "Code intelligence" }))
    const before = mockGetStatus.mock.calls.length
    fireEvent.click(screen.getByRole("button", { name: "Check" }))
    await waitFor(() =>
      expect(mockGetStatus.mock.calls.length).toBe(before + 1)
    )
    expect(mockGetStatus).toHaveBeenLastCalledWith(null)
    expect(mockSetSettings).not.toHaveBeenCalled()
  })

  it("shows detected and unconfigured on separate LSP rows", async () => {
    renderSettings()
    const rustBox = await screen.findByRole("checkbox", { name: "Rust" })
    const rust = rustBox.closest("li")
    const go = screen.getByRole("checkbox", { name: "Go" }).closest("li")
    if (!rust || !go) throw new Error("language row missing")

    expect(within(rust).getByText("Detected")).toBeInTheDocument()
    expect(within(rust).getByText("Unconfigured")).toBeInTheDocument()
    expect(within(rust).queryByText("Not installed")).not.toBeInTheDocument()

    expect(within(go).getByText("Not detected")).toBeInTheDocument()
    expect(within(go).getByText("Unconfigured")).toBeInTheDocument()
    expect(within(go).queryByText("Not installed")).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: /^LSP\b/ })).toHaveTextContent(
      "Unconfigured"
    )
  })

  it("does not relabel a handshake failure as not installed", async () => {
    mockGetStatus.mockResolvedValue(
      defaultStatus({
        providers: [
          provider("lsp", { configured: true, discovery: "unconfigured" }),
          provider("codegraph", {
            configured: true,
            discovery: "global",
            runtime: "stopped",
          }),
          provider("serena", {
            configured: true,
            discovery: "missing",
            install: "idle",
            runtime: "handshake_failed",
            last_error: "broken pipe",
          }),
        ],
      })
    )
    renderSettings()
    const serena = await screen.findByRole("button", { name: /^Serena\b/ })
    expect(serena).toHaveTextContent("Handshake failed")
    expect(serena).not.toHaveTextContent("Not installed")
    expect(screen.queryByText("Not installed")).not.toBeInTheDocument()

    fireEvent.click(serena)
    expect(screen.getByText("broken pipe")).toBeInTheDocument()
    expect(screen.getByText("View log")).toBeInTheDocument()
    expect(
      screen.queryByRole("button", { name: "Apply" })
    ).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "Retry" })).toBeDisabled()

    fireEvent.click(screen.getByRole("switch", { name: "Code intelligence" }))
    mockRetry.mockResolvedValue(
      defaultStatus({
        providers: [
          provider("lsp", { configured: true, discovery: "unconfigured" }),
          provider("codegraph", {
            configured: true,
            discovery: "global",
            runtime: "stopped",
          }),
          provider("serena", {
            configured: true,
            discovery: "missing",
            install: "idle",
            runtime: "handshake_failed",
            last_error: "broken pipe",
          }),
        ],
      })
    )
    fireEvent.click(screen.getByRole("button", { name: "Retry" }))
    await waitFor(() => expect(mockRetry).toHaveBeenCalledTimes(1))
    expect(mockRetry).toHaveBeenCalledWith(null)
    expect(mockSetSettings).not.toHaveBeenCalled()
  })

  it("does not relabel an install failure as not installed", async () => {
    mockGetStatus.mockResolvedValue(
      defaultStatus({
        providers: [
          provider("lsp", { configured: true, discovery: "unconfigured" }),
          provider("codegraph", {
            configured: true,
            discovery: "managed",
            runtime: "connected",
          }),
          provider("serena", {
            configured: true,
            discovery: "missing",
            install: "failed",
            runtime: "stopped",
            last_error: "checksum mismatch",
          }),
        ],
      })
    )
    renderSettings()
    const serena = await screen.findByRole("button", { name: /^Serena\b/ })
    expect(serena).toHaveTextContent("Install failed")
    expect(serena).not.toHaveTextContent("Not installed")
    expect(screen.queryByText("Not installed")).not.toBeInTheDocument()
    expect(
      screen.getByRole("button", { name: /^CodeGraph\b/ })
    ).toHaveTextContent("Connected")
  })

  it("shows not installed when discovery is missing and nothing has failed", async () => {
    renderSettings()
    const codegraph = await screen.findByRole("button", {
      name: /^CodeGraph\b/,
    })
    expect(codegraph).toHaveTextContent("Not installed")
    expect(codegraph).not.toHaveTextContent("Handshake failed")
    expect(codegraph).not.toHaveTextContent("Install failed")
  })

  it("rejects a relative path and shell characters without saving", async () => {
    renderSettings()
    fireEvent.click(await screen.findByRole("button", { name: /^CodeGraph\b/ }))
    const input = screen.getByRole("textbox", { name: "Binary path" })
    fireEvent.change(input, { target: { value: "bin/codegraph" } })
    fireEvent.click(screen.getByRole("button", { name: "Save and apply" }))
    expect(mockSetSettings).not.toHaveBeenCalled()
    expect(screen.getByRole("alert")).toHaveTextContent(/relative paths/i)

    fireEvent.change(input, { target: { value: "/tmp/codegraph;rm" } })
    fireEvent.click(screen.getByRole("button", { name: "Save and apply" }))
    expect(mockSetSettings).not.toHaveBeenCalled()
    expect(screen.getByRole("alert")).toHaveTextContent(/shell characters/i)
  })

  it("saves the new config and reloads status", async () => {
    const initial = defaultStatus()
    mockGetStatus.mockResolvedValue(initial)
    renderSettings()

    expect(await screen.findByRole("button", { name: "Save" })).toBeDisabled()

    fireEvent.click(screen.getByRole("switch", { name: "Code intelligence" }))
    fireEvent.click(screen.getByRole("switch", { name: "Serena" }))

    const rust = screen.getByRole("checkbox", { name: "Rust" })
    expect(rust).toBeChecked()
    fireEvent.click(rust)
    expect(rust).not.toBeChecked()

    fireEvent.change(
      screen.getByRole("spinbutton", { name: "Maximum running at once" }),
      { target: { value: "99" } }
    )

    fireEvent.click(screen.getByRole("button", { name: /^CodeGraph\b/ }))
    fireEvent.change(screen.getByRole("textbox", { name: "Binary path" }), {
      target: { value: "/opt/my tools/codegraph" },
    })
    expect(
      screen.getByText('"/opt/my tools/codegraph" serve --mcp')
    ).toBeInTheDocument()
    expect(
      screen.queryByRole("textbox", { name: "Command" })
    ).not.toBeInTheDocument()

    const before = mockGetStatus.mock.calls.length
    fireEvent.click(screen.getByRole("button", { name: "Save and apply" }))

    await waitFor(() => {
      expect(mockSetSettings).toHaveBeenCalledTimes(1)
      expect(mockGetStatus.mock.calls.length).toBeGreaterThan(before)
    })
    expect(mockSetSettings).toHaveBeenCalledWith({
      enabled: true,
      lsp: {
        enabled: true,
        languages: ["go", "python", "typescript", "cpp"],
        max_concurrent: 8,
        auto_install: true,
      },
      codegraph: {
        enabled: true,
        auto_install: true,
        binary_path: "/opt/my tools/codegraph",
      },
      serena: {
        enabled: true,
        auto_install: true,
        command: null,
        version: "v1.7.0",
        context: "codex",
        modes: ["interactive", "editing", "planning"],
      },
    })
    const payload = mockSetSettings.mock.calls[0][0]
    expect(payload.lsp).not.toHaveProperty("auto_attach")
    expect(payload.lsp).not.toHaveProperty("checked")
    expect(payload.lsp).not.toHaveProperty("custom")
    expect(payload).not.toHaveProperty("migration_report")
  })

  it("shows Serena's official argv as read-only text and links to the docs", async () => {
    renderSettings()
    fireEvent.click(await screen.findByRole("button", { name: /^Serena\b/ }))

    expect(screen.getByRole("combobox", { name: "Context" })).toHaveValue(
      "codex"
    )
    expect(
      screen.getByText("Modes: interactive, editing, planning")
    ).toBeInTheDocument()
    expect(
      screen.getByText(
        "uvx --from git+https://github.com/oraios/serena@v1.7.0 serena start-mcp-server --context codex --project /work/codeg --mode interactive --mode editing --mode planning --enable-web-dashboard false --open-web-dashboard false"
      )
    ).toBeInTheDocument()
    const docs = screen.getByRole("link", { name: "Official documentation" })
    expect(docs).toHaveAttribute("href", "https://github.com/oraios/serena")
    expect(docs).toHaveAttribute("target", "_blank")
    expect(
      screen.queryByRole("textbox", { name: "Command" })
    ).not.toBeInTheDocument()
  })

  it("uses a resolved Serena command instead of uvx", async () => {
    mockGetStatus.mockResolvedValue(
      defaultStatus({
        providers: [
          provider("lsp", { configured: true, discovery: "unconfigured" }),
          provider("codegraph", { configured: true, discovery: "missing" }),
          provider("serena", {
            configured: false,
            discovery: "global",
            resolved_command: "/usr/local/bin/serena",
          }),
        ],
      })
    )
    renderSettings()
    fireEvent.click(await screen.findByRole("button", { name: /^Serena\b/ }))
    expect(
      screen.getByText(/^\/usr\/local\/bin\/serena start-mcp-server/)
    ).toBeInTheDocument()
    expect(screen.queryByText(/uvx/)).not.toBeInTheDocument()
  })

  it("does not show the uvx install command when auto-install is off", async () => {
    const config = structuredClone(DEFAULT_CONFIG)
    config.serena.auto_install = false
    mockGetStatus.mockResolvedValue(defaultStatus({ config }))
    renderSettings()
    fireEvent.click(await screen.findByRole("button", { name: /^Serena\b/ }))
    expect(screen.getByText(/^serena start-mcp-server/)).toBeInTheDocument()
    expect(screen.queryByText(/uvx/)).not.toBeInTheDocument()
  })

  it("shows a migration report from status without writing settings", async () => {
    mockGetStatus.mockResolvedValue(
      defaultStatus({
        migration_report: "Stashed custom LSP servers.",
      })
    )
    renderSettings()
    expect(
      await screen.findByText(/Stashed custom LSP servers/)
    ).toBeInTheDocument()
    expect(mockSetSettings).not.toHaveBeenCalled()
    expect(mockGetSettings).not.toHaveBeenCalled()
  })
})
