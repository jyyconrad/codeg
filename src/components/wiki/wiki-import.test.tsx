import {
  render,
  screen,
  fireEvent,
  within,
  cleanup,
} from "@testing-library/react"
import { NextIntlClientProvider } from "next-intl"
import { afterEach, beforeEach, expect, it, vi } from "vitest"
import messages from "@/i18n/messages/en.json"
import { WikiImportButton } from "./wiki-import-dialog"
import { WikiDataProvider } from "./wiki-data"

const imports = vi.hoisted(() => ({ text: vi.fn(), files: vi.fn() }))
vi.mock("@/lib/transport", () => ({
  getTransport: () => ({
    call: async (command: string) =>
      command === "wiki_get_overview"
        ? { active_job_count: 0 }
        : { enabled: true },
    subscribe: async () => () => {},
  }),
}))
vi.mock("@/lib/platform", () => ({ isLocalDesktop: () => false }))
vi.mock("@/components/shared/directory-browser-dialog", () => ({
  DirectoryBrowserDialog: () => null,
}))
vi.mock("@/lib/wiki-api", () => ({
  wikiImportText: imports.text,
  wikiImportFiles: imports.files,
  wikiImportDirectory: vi.fn(),
  wikiImportLocalSessions: vi.fn(),
}))
vi.mock("@/lib/api", () => ({ scanImportableSessions: vi.fn() }))
beforeEach(() => {
  imports.files.mockReset()
  imports.text.mockReset()
})
afterEach(cleanup)

async function openImport() {
  window.history.replaceState({}, "", "/?wikiView=overview")
  render(
    <NextIntlClientProvider locale="en" messages={messages}>
      <WikiDataProvider>
        <WikiImportButton />
      </WikiDataProvider>
    </NextIntlClientProvider>
  )
  fireEvent.click(
    screen.getByRole("button", { name: messages.Wiki.v2.addSources })
  )
  return screen.findByRole("dialog")
}
function selectFile(dialog: HTMLElement, filename: string) {
  const file = new File(["fixture content"], filename, {
    type: "application/pdf",
  })
  // jsdom's File omits arrayBuffer; browsers provide this standard method.
  Object.defineProperty(file, "arrayBuffer", {
    value: async () => new Uint8Array([1, 2, 3]).buffer,
  })
  fireEvent.change(
    within(dialog).getByLabelText(messages.Wiki.v2.chooseFiles, {
      exact: false,
    }),
    { target: { files: [file] } }
  )
}

it("keeps single-file batch results and opens the existing material after a duplicate import", async () => {
  imports.files.mockResolvedValue({
    request_id: "request-1",
    results: [
      {
        filename: "Saved material.pdf",
        request_id: "request-1",
        source: {
          id: "source-1",
          source_title: "Saved material",
          eligibility: "ready",
        },
        duplicate: true,
        status: "duplicate",
      },
    ],
    succeeded: 0,
    duplicates: 1,
    failed: 0,
  })
  const dialog = await openImport()
  selectFile(dialog, "Saved material.pdf")
  fireEvent.click(
    within(dialog).getByRole("button", { name: messages.Wiki.v2.addSources })
  )
  expect(await within(dialog).findByText("Saved material.pdf")).toBeVisible()
  expect(
    within(dialog).getByText(messages.Wiki.v2.importStatus.duplicate)
  ).toBeVisible()
  fireEvent.click(
    within(dialog).getByRole("button", { name: messages.Wiki.v2.read })
  )
  expect(window.location.search).toContain("wikiSource=source-1")
  fireEvent.click(
    screen.getByRole("button", { name: messages.Wiki.v2.addSources })
  )
  expect(
    within(await screen.findByRole("dialog")).getByText("Saved material.pdf")
  ).toBeVisible()
})

it("reports a failed extraction with its error and a material-information link instead of claiming it was added", async () => {
  imports.files.mockResolvedValue({
    request_id: "request-failed",
    results: [
      {
        filename: "Scanned.pdf",
        request_id: "request-failed",
        source: { id: "source-failed", eligibility: "failed" },
        duplicate: false,
        status: "failed",
        error: "PDF contains no extractable text",
      },
    ],
    succeeded: 0,
    failed: 1,
    duplicates: 0,
  })
  const dialog = await openImport()
  selectFile(dialog, "Scanned.pdf")
  fireEvent.click(
    within(dialog).getByRole("button", { name: messages.Wiki.v2.addSources })
  )
  expect(
    await within(dialog).findByText(messages.Wiki.v2.noExtractedText)
  ).toBeVisible()
  expect(
    within(dialog).getByText("PDF contains no extractable text")
  ).not.toBeVisible()
  fireEvent.click(within(dialog).getByText(messages.Wiki.v2.technicalDetails))
  expect(
    within(dialog).getByText("PDF contains no extractable text")
  ).toBeVisible()
  expect(
    within(dialog).getByText(messages.Wiki.v2.importStatus.failed)
  ).toBeVisible()
  expect(within(dialog).getByRole("status")).toHaveTextContent(
    "Added 0 · Already saved 0 · Needs attention 1"
  )
  fireEvent.click(
    within(dialog).getByRole("button", {
      name: messages.Wiki.v2.sourceTabs.info,
    })
  )
  expect(window.location.search).toContain("wikiSource=source-failed")
})

it("counts an unsuccessful pasted-text extraction as failed even when the material already exists", async () => {
  imports.text.mockResolvedValue({
    id: "text-failed",
    source_title: "Saved text",
    eligibility: "failed",
    duplicate: true,
    warnings: ["Text extraction failed"],
  })
  const dialog = await openImport()
  fireEvent.click(
    within(dialog).getByRole("button", {
      name: messages.Wiki.v2.importModes.text,
    })
  )
  fireEvent.change(
    within(dialog).getByRole("textbox", { name: messages.Wiki.v2.pasteText }),
    { target: { value: "some text" } }
  )
  fireEvent.click(
    within(dialog).getByRole("button", { name: messages.Wiki.v2.addSources })
  )
  expect(
    await within(dialog).findByText(messages.Wiki.v2.noExtractedText)
  ).toBeVisible()
  expect(within(dialog).getByRole("status")).toHaveTextContent(
    "Added 0 · Already saved 0 · Needs attention 1"
  )
  expect(
    within(dialog).getByRole("button", {
      name: messages.Wiki.v2.sourceTabs.info,
    })
  ).toBeEnabled()
})

it.each(["files", "text"] as const)(
  "keeps partial %s extraction as added while counting it for review",
  async (mode) => {
    const source = {
      id: "partial-source",
      source_title: "Partial material",
      extraction_status: "partial",
      eligibility: "awaiting-acceptance",
      duplicate: false,
    }
    imports.files.mockResolvedValue({
      request_id: "partial-request",
      results: [
        {
          filename: "Partial.pdf",
          request_id: "partial-request",
          source,
          duplicate: false,
          status: "succeeded",
        },
      ],
      succeeded: 1,
      failed: 0,
      duplicates: 0,
    })
    imports.text.mockResolvedValue(source)
    const dialog = await openImport()
    if (mode === "files") selectFile(dialog, "Partial.pdf")
    else {
      fireEvent.click(
        within(dialog).getByRole("button", {
          name: messages.Wiki.v2.importModes.text,
        })
      )
      fireEvent.change(
        within(dialog).getByRole("textbox", {
          name: messages.Wiki.v2.pasteText,
        }),
        { target: { value: "partial material" } }
      )
    }
    fireEvent.click(
      within(dialog).getByRole("button", { name: messages.Wiki.v2.addSources })
    )
    expect(
      await within(dialog).findByText(messages.Wiki.v2.partialHint)
    ).toBeVisible()
    expect(
      within(dialog).getByText(messages.Wiki.v2.importStatus.succeeded)
    ).toBeVisible()
    expect(within(dialog).getByRole("status")).toHaveTextContent(
      "Added 1 · Already saved 0 · Needs attention 1"
    )
    expect(
      within(dialog).getByText(`${messages.Wiki.v2.partial} · 1`)
    ).toBeVisible()
    expect(
      within(dialog).queryByText(messages.Wiki.v2.importStatus.failed)
    ).not.toBeInTheDocument()
    expect(
      within(dialog).getByRole("button", { name: messages.Wiki.v2.read })
    ).toBeEnabled()
  }
)
