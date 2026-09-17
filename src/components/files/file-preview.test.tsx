import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

import type { FileWorkspaceTab } from "@/contexts/workspace-context"

const {
  mockDetect,
  mockStartWatch,
  mockIsDesktop,
  mockIsRemote,
} = vi.hoisted(() => ({
  mockDetect: vi.fn(),
  mockStartWatch: vi.fn(),
  mockIsDesktop: vi.fn(),
  mockIsRemote: vi.fn(),
}))

vi.mock("next-intl", () => ({
  useTranslations: () => (key: string) => key,
}))

vi.mock("@/lib/api", () => ({
  officecliDetect: mockDetect,
  startOfficeWatch: mockStartWatch,
  stopOfficeWatch: vi.fn().mockResolvedValue(undefined),
  openSettingsWindow: vi.fn(),
  readWorkspaceFileBase64: vi.fn(),
  readFileBase64: vi.fn(),
  readSpreadsheetPreview: vi.fn(),
}))

vi.mock("@/lib/transport", () => ({
  isDesktop: mockIsDesktop,
  isRemoteDesktopMode: mockIsRemote,
  getServerBaseUrl: () => "https://srv.example",
}))

vi.mock("@/components/files/image-preview", () => ({
  ImagePreview: () => <div data-testid="image-preview" />,
}))

vi.mock("@/components/files/html-preview", () => ({
  HtmlPreview: () => <div data-testid="html-preview" />,
}))

vi.mock("@/components/files/markdown-document-preview", () => ({
  MarkdownDocumentPreview: () => <div data-testid="markdown-preview" />,
}))

vi.mock("@/components/files/office-js-preview", () => ({
  OfficeJsPreview: ({ kind }: { kind: string }) => (
    <div data-testid="office-js-preview" data-kind={kind} />
  ),
}))

vi.mock("@/components/files/spreadsheet-preview", () => ({
  SpreadsheetPreview: () => <div data-testid="spreadsheet-preview" />,
}))

vi.mock("@/hooks/use-workspace-state-store", () => ({
  useWorkspaceStateStore: () => ({
    subscribeEnvelopes: () => () => {},
  }),
}))

vi.mock("@/stores/app-workspace-store", () => ({
  useAppWorkspaceStore: () => [],
}))

import { FilePreview } from "./file-preview"

function tab(overrides: Partial<FileWorkspaceTab>): FileWorkspaceTab {
  return {
    id: "tab-1",
    kind: "file",
    folderId: null,
    title: "file",
    description: "/repo/file",
    path: "/repo/file",
    language: "plaintext",
    content: "",
    loading: false,
    ...overrides,
  }
}

function renderPreview(path: string, extra?: Partial<FileWorkspaceTab> & { isPreview?: boolean }) {
  const { isPreview = true, ...rest } = extra ?? {}
  const current = tab({ path, title: path, description: path, ...rest })
  return render(
    <FilePreview
      path={path}
      content={current.content}
      rootPath="/repo"
      relPath={path.split("/").pop() ?? path}
      language={current.language}
      isPreview={isPreview}
      tab={current}
    />
  )
}

beforeEach(() => {
  vi.clearAllMocks()
  mockIsDesktop.mockReturnValue(true)
  mockIsRemote.mockReturnValue(false)
  mockDetect.mockResolvedValue({
    installed: false,
    version: null,
    path: null,
    runtimeError: null,
  })
  mockStartWatch.mockResolvedValue({ port: 1, cap: "c" })
})

afterEach(() => cleanup())

describe("FilePreview dispatch", () => {
  it("renders the existing image preview for pictures", () => {
    renderPreview("/repo/a.png", { language: "image" })
    expect(screen.getByTestId("image-preview")).toBeTruthy()
  })

  it("renders markdown through the existing document preview", () => {
    renderPreview("/repo/a.md", { language: "markdown", content: "# Hi" })
    expect(screen.getByTestId("markdown-preview")).toBeTruthy()
  })

  it("renders html through the existing sandboxed preview", () => {
    renderPreview("/repo/a.html", { language: "html", content: "<p>x</p>" })
    expect(screen.getByTestId("html-preview")).toBeTruthy()
  })
})

describe("FilePreview office-cli fallback", () => {
  it("uses officecli watch for a docx when the CLI is installed", async () => {
    mockDetect.mockResolvedValue({
      installed: true,
      version: "1.0",
      path: "/bin/officecli",
      runtimeError: null,
    })
    renderPreview("/repo/a.docx", { language: "office" })
    await waitFor(() =>
      expect(screen.getByTitle("officePreviewTitle")).toBeTruthy()
    )
    expect(mockStartWatch).toHaveBeenCalled()
    expect(screen.queryByTestId("office-js-preview")).toBeNull()
  })

  it("falls back to OfficeJsPreview for a docx when officecli is missing", async () => {
    renderPreview("/repo/a.docx", { language: "office" })
    expect(await screen.findByTestId("office-js-preview")).toHaveAttribute(
      "data-kind",
      "docx"
    )
    expect(mockStartWatch).not.toHaveBeenCalled()
  })

  it("never starts a watch for pdf", async () => {
    mockDetect.mockResolvedValue({
      installed: true,
      version: "1.0",
      path: "/bin/officecli",
      runtimeError: null,
    })
    renderPreview("/repo/a.pdf", { language: "pdf" })
    expect(await screen.findByTestId("office-js-preview")).toHaveAttribute(
      "data-kind",
      "pdf"
    )
    expect(mockStartWatch).not.toHaveBeenCalled()
  })

  it("uses SpreadsheetPreview for xlsx when officecli is missing", async () => {
    renderPreview("/repo/a.xlsx", { language: "spreadsheet" })
    expect(await screen.findByTestId("spreadsheet-preview")).toBeTruthy()
    expect(mockStartWatch).not.toHaveBeenCalled()
  })

  it("uses officecli watch for xlsx when the CLI is installed", async () => {
    mockDetect.mockResolvedValue({
      installed: true,
      version: "1.0",
      path: "/bin/officecli",
      runtimeError: null,
    })
    renderPreview("/repo/a.xlsx", { language: "spreadsheet" })
    await waitFor(() =>
      expect(screen.getByTitle("officePreviewTitle")).toBeTruthy()
    )
    expect(screen.queryByTestId("spreadsheet-preview")).toBeNull()
  })

  it("skips officecli watch on remote desktop and uses the frontend engine", async () => {
    mockIsRemote.mockReturnValue(true)
    mockDetect.mockResolvedValue({
      installed: true,
      version: "1.0",
      path: "/bin/officecli",
      runtimeError: null,
    })
    renderPreview("/repo/a.docx", { language: "office" })
    expect(await screen.findByTestId("office-js-preview")).toBeTruthy()
    expect(mockStartWatch).not.toHaveBeenCalled()
    expect(screen.queryByText("officeRemoteDesktopUnsupported")).toBeNull()
  })
})
