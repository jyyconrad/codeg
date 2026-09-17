import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

const mockRead = vi.hoisted(() => vi.fn())

vi.mock("next-intl", () => ({
  useTranslations: () => (key: string, values?: Record<string, unknown>) => {
    if (key === "spreadsheetRows" && values) {
      return `rows ${values.from}-${values.to}/${values.total}`
    }
    return key
  },
}))

vi.mock("@/lib/api", () => ({
  readSpreadsheetPreview: mockRead,
}))

vi.mock("@/hooks/use-preview-file-changes", () => ({
  usePreviewFileChanges: () => {},
}))

import { SpreadsheetPreview } from "./spreadsheet-preview"

const PAGE = {
  path: "data.csv",
  sheets: [{ name: "data.csv", rowCount: 201, columnCount: 2 }],
  sheet: "data.csv",
  header: ["h1", "h2"],
  rowOffset: 0,
  rowLimit: 100,
  colOffset: 0,
  colLimit: 64,
  totalRows: 201,
  totalColumns: 2,
  rows: Array.from({ length: 100 }, (_, i) => [`r${i + 1}`, `v${i + 1}`]),
  eof: false,
}

beforeEach(() => {
  vi.clearAllMocks()
  mockRead.mockResolvedValue(PAGE)
})
afterEach(() => cleanup())

describe("SpreadsheetPreview", () => {
  it("renders header cells as text and shows the 1-based row range", async () => {
    render(
      <SpreadsheetPreview
        path="/repo/data.csv"
        rootPath="/repo"
        relPath="data.csv"
      />
    )
    expect(await screen.findByText("h1")).toBeTruthy()
    expect(screen.getByText("rows 2-101/201")).toBeTruthy()
    expect(screen.getByText("r1")).toBeTruthy()
    expect(mockRead).toHaveBeenCalledWith(
      expect.objectContaining({
        rootPath: "/repo",
        path: "data.csv",
        rowOffset: 0,
        rowLimit: 100,
      })
    )
  })

  it("requests the next page without downloading the whole sheet", async () => {
    mockRead
      .mockResolvedValueOnce(PAGE)
      .mockResolvedValueOnce({
        ...PAGE,
        rowOffset: 100,
        rows: Array.from({ length: 100 }, (_, i) => [
          `r${i + 101}`,
          `v${i + 101}`,
        ]),
        eof: true,
      })

    render(
      <SpreadsheetPreview
        path="/repo/data.csv"
        rootPath="/repo"
        relPath="data.csv"
      />
    )
    await screen.findByText("h1")
    fireEvent.click(screen.getByLabelText("spreadsheetNextPage"))
    await waitFor(() =>
      expect(mockRead).toHaveBeenCalledWith(
        expect.objectContaining({ rowOffset: 100, rowLimit: 100 })
      )
    )
  })
})
