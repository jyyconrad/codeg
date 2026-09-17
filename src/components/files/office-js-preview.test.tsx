import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

const { mockInit, mockPreview, mockDestroy, mockRead } = vi.hoisted(() => {
  const mockPreview = vi.fn().mockResolvedValue(undefined)
  const mockDestroy = vi.fn()
  const mockInit = vi.fn(() => ({ preview: mockPreview, destroy: mockDestroy }))
  const mockRead = vi.fn()
  return { mockInit, mockPreview, mockDestroy, mockRead }
})

vi.mock("next-intl", () => ({
  useTranslations: () => (key: string) => key,
}))

vi.mock("@/lib/office-file-bytes", () => ({
  readOfficeFileBytesWithRetry: mockRead,
}))

vi.mock("@/hooks/use-preview-file-changes", () => ({
  usePreviewFileChanges: () => {},
}))

vi.mock("@js-preview/docx", () => ({
  default: { init: mockInit },
}))

vi.mock("@js-preview/docx/lib/index.css", () => ({}))

vi.mock("@js-preview/pdf", () => ({
  default: { init: mockInit },
}))

vi.mock("pptx-preview", () => ({
  init: mockInit,
}))

import { OfficeJsPreview } from "./office-js-preview"

beforeEach(() => {
  vi.clearAllMocks()
  mockRead.mockResolvedValue(new ArrayBuffer(8))
  mockPreview.mockResolvedValue(undefined)
})
afterEach(() => cleanup())

describe("OfficeJsPreview", () => {
  it("inits, previews bytes, and destroys on unmount", async () => {
    const { unmount } = render(
      <OfficeJsPreview
        kind="docx"
        path="/repo/a.docx"
        rootPath="/repo"
        relPath="a.docx"
      />
    )
    await waitFor(() => expect(mockInit).toHaveBeenCalled())
    expect(mockPreview).toHaveBeenCalled()
    unmount()
    expect(mockDestroy).toHaveBeenCalled()
  })

  it("shows a retryable error when the file cannot be read", async () => {
    mockRead.mockRejectedValue(new Error("too large"))
    render(
      <OfficeJsPreview
        kind="pdf"
        path="/repo/a.pdf"
        rootPath="/repo"
        relPath="a.pdf"
      />
    )
    expect(await screen.findAllByText("officeJsPreviewFailed")).not.toHaveLength(
      0
    )
    expect(screen.getByText("officeJsPreviewRetry")).toBeTruthy()
  })
})
