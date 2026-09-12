import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import { NextIntlClientProvider } from "next-intl"
import { beforeEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/api", () => ({
  getCodeIntelStatus: vi.fn(),
  setCodeIntelSettings: vi.fn(),
}))

vi.mock("@/stores/app-workspace-store", () => ({
  useAppWorkspaceStore: (sel: (s: never) => unknown) =>
    sel({ activeFolderId: null, allFolders: [] } as never),
}))

import * as api from "@/lib/api"
import {
  getCodeIntelStatus,
  setCodeIntelSettings,
  type CodeIntelStatus,
} from "@/lib/api"
import enMessages from "@/i18n/messages/en.json"
import { CodeIntelligenceSettings } from "./code-intelligence-settings"

const mockGetStatus = vi.mocked(getCodeIntelStatus)
const mockSetSettings = vi.mocked(setCodeIntelSettings)

function defaultStatus(
  overrides: Partial<CodeIntelStatus> = {}
): CodeIntelStatus {
  return {
    config: {
      enabled: false,
      codegraph: { enabled: true, binary_path: null },
      lsp: {
        auto_attach: true,
        max_concurrent: 2,
        checked: ["rust-analyzer"],
        custom: [],
      },
    },
    codegraph_binary: null,
    codegraph_indexed: false,
    cwd: null,
    lsp_servers: [
      {
        id: "rust-analyzer",
        language: "Rust",
        binary: "rust-analyzer",
        binary_on_path: false,
        checked: true,
        language_detected: false,
        default_checked: true,
        custom: false,
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

describe("CodeIntelligenceSettings", () => {
  beforeEach(() => {
    mockGetStatus.mockReset()
    mockSetSettings.mockReset()
    mockGetStatus.mockResolvedValue(defaultStatus())
    mockSetSettings.mockImplementation(async (settings) => settings)
  })

  it("renders master switch unchecked by default", async () => {
    renderSettings()
    const sw = await screen.findByRole("switch", {
      name: /enable code intelligence/i,
    })
    expect(sw).toHaveAttribute("data-state", "unchecked")
  })

  it("lists rust-analyzer as checked and shows PATH miss", async () => {
    renderSettings()
    expect(
      (await screen.findAllByText(/rust-analyzer/i)).length
    ).toBeGreaterThan(0)
    expect(screen.getByText("not on PATH")).toBeInTheDocument()
    expect(
      screen.getByRole("checkbox", { name: /rust-analyzer/i })
    ).toHaveAttribute("data-state", "checked")
  })

  it("does not call install APIs", async () => {
    renderSettings()
    await screen.findByText(/npm i -g @colbymchenry\/codegraph/i)
    for (const [name, value] of Object.entries(api)) {
      if (!/install/i.test(name)) continue
      expect(value, name).not.toHaveBeenCalled()
    }
  })

  it("disables the CodeGraph switch while master is off", async () => {
    renderSettings()
    const cg = await screen.findByRole("switch", { name: /^codegraph$/i })
    expect(cg).toBeDisabled()
  })

  it("saves then reloads status", async () => {
    const initial = defaultStatus()
    mockGetStatus.mockResolvedValue(initial)
    renderSettings()

    const sw = await screen.findByRole("switch", {
      name: /enable code intelligence/i,
    })
    fireEvent.click(sw)

    fireEvent.click(screen.getByRole("button", { name: /^save$/i }))

    await waitFor(() =>
      expect(mockSetSettings).toHaveBeenCalledWith({
        ...initial.config,
        enabled: true,
      })
    )
    await waitFor(() =>
      expect(mockGetStatus.mock.calls.length).toBeGreaterThan(1)
    )
  })
})
