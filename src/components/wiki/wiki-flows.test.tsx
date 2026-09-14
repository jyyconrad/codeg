import {
  render,
  screen,
  fireEvent,
  waitFor,
  cleanup,
} from "@testing-library/react"
import { NextIntlClientProvider } from "next-intl"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import messages from "@/i18n/messages/en.json"
import { WikiDataProvider } from "./wiki-data"
import { WikiJobsView } from "./wiki-jobs-view"
import { WikiSourcesView } from "./wiki-sources-view"
import { WikiNoteReader } from "./wiki-note-reader"
const backend = vi.hoisted(() => ({
  call: vi.fn(),
  compile: vi.fn(),
  readNote: vi.fn(),
}))
vi.mock("@/lib/transport", () => ({
  getTransport: () => ({ call: backend.call, subscribe: async () => () => {} }),
}))
vi.mock("@/lib/wiki-api", () => ({
  wikiReadNote: backend.readNote,
  wikiCompileNow: backend.compile,
  wikiCancelJob: vi.fn(),
  wikiRetryJob: vi.fn(),
  wikiAcceptExtraction: vi.fn(),
  wikiReextract: vi.fn(),
}))
vi.mock("@/components/ai-elements/markdown-link", () => ({
  MarkdownLink: ({
    children,
    href,
  }: {
    children: React.ReactNode
    href: string
  }) => <a href={href}>{children}</a>,
}))
function wrapper({ children }: { children: React.ReactNode }) {
  return (
    <NextIntlClientProvider locale="en" messages={messages}>
      <WikiDataProvider>{children}</WikiDataProvider>
    </NextIntlClientProvider>
  )
}
beforeEach(() => {
  backend.call.mockReset()
  backend.compile.mockReset()
  backend.readNote.mockReset()
  window.history.replaceState({}, "", "/")
})
afterEach(cleanup)
describe("Wiki workbench flows", () => {
  it("shows a manual organization error even without a selected job", async () => {
    backend.call.mockImplementation((command: string) =>
      Promise.resolve(
        command === "wiki_get_overview"
          ? { active_job_count: 0, pending_memory_count: 1 }
          : command === "get_wiki_settings"
            ? { enabled: true, synthesize: { enabled: true } }
            : { items: [], total: 0 }
      )
    )
    backend.compile.mockRejectedValue(new Error("Connection unavailable"))
    render(<WikiJobsView />, { wrapper })
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: messages.Wiki.v2.organize })
      ).toBeEnabled()
    )
    fireEvent.click(
      screen.getByRole("button", { name: messages.Wiki.v2.organize })
    )
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Connection unavailable"
    )
  })
  it("opens extracted source content directly and retains related notes in their own section", async () => {
    window.history.replaceState({}, "", "/?wikiView=sources&wikiSource=s1")
    const source = {
      id: "s1",
      source_title: "Design document",
      source_kind: "document",
      raw_path: "raw/s1.md",
    }
    backend.call.mockImplementation((command: string) =>
      Promise.resolve(
        command === "wiki_get_overview"
          ? { active_job_count: 0 }
          : command === "wiki_list_sources_page"
            ? { items: [source], total: 1 }
            : {
                source,
                body: "## Actual original content\n\nReadable text",
                raw: "ORIGINAL",
                format_warning: false,
                related_notes: [],
              }
      )
    )
    render(<WikiSourcesView />, { wrapper })
    expect(
      await screen.findByRole("heading", { name: "Actual original content" })
    ).toBeVisible()
    expect(
      screen.queryByText(messages.Wiki.v2.noRelated)
    ).not.toBeInTheDocument()
    fireEvent.click(
      screen.getByRole("button", { name: messages.Wiki.v2.sourceTabs.related })
    )
    expect(screen.getByText(messages.Wiki.v2.noRelated)).toBeVisible()
  })
  it("keeps the current note open when a linked note is missing", async () => {
    const note = {
      note_id: "n1",
      path: "work/a.md",
      title: "Existing note",
      summary: "",
      type: "work-record",
      updated_at: null,
      project_ids: [],
      source_ids: [],
      evidence_level: null,
    }
    backend.call.mockImplementation((command: string) =>
      Promise.resolve(
        command === "wiki_get_overview"
          ? { active_job_count: 0 }
          : {
              note,
              body: "[[work/missing|Missing note]]",
              source: "source",
              format_warning: false,
              sources: [],
              headings: [],
            }
      )
    )
    backend.readNote.mockRejectedValue(new Error("missing"))
    render(<WikiNoteReader path="work/a.md" />, { wrapper })
    fireEvent.click(await screen.findByRole("link", { name: "Missing note" }))
    expect(await screen.findByRole("alert")).toHaveTextContent(
      messages.Wiki.v2.missingLink
    )
    expect(screen.getByRole("heading", { name: "Existing note" })).toBeVisible()
    expect(window.location.search).not.toContain("missing")
  })
  it("preserves source paths and disables opening a source that no longer exists", async () => {
    backend.call.mockImplementation((command: string) =>
      Promise.resolve(
        command === "wiki_get_overview"
          ? { active_job_count: 0 }
          : {
              note: {
                note_id: "n1",
                path: "work/a.md",
                title: "Source checks",
                type: "work-record",
                summary: "",
                updated_at: null,
                project_ids: [],
                source_ids: [],
                evidence_level: null,
              },
              body: "Readable note",
              source: "Readable note",
              format_warning: false,
              headings: [],
              sources: [
                {
                  source_id: "missing",
                  title: "Removed source",
                  path: "/original/project/removed.md",
                  start_line: 3,
                  end_line: 5,
                  excerpt: null,
                  availability: "missing",
                },
                {
                  source_id: "exists",
                  title: "Existing source",
                  path: "/original/project/existing.md",
                  source_url: "https://example.com/project/design",
                  start_line: null,
                  end_line: null,
                  excerpt: null,
                  availability: "available",
                },
              ],
            }
      )
    )
    render(<WikiNoteReader path="work/a.md" />, { wrapper })
    const missing = await screen.findByRole("button", {
      name: "Removed source",
    })
    expect(missing).toBeDisabled()
    expect(screen.getByText("/original/project/removed.md")).toBeVisible()
    expect(screen.getByText("https://example.com/project/design")).toBeVisible()
    expect(
      screen.getByText(messages.Wiki.v2.sourceAvailability.missing)
    ).toBeVisible()
    fireEvent.click(missing)
    expect(window.location.search).not.toContain("wikiSource=missing")
    fireEvent.click(screen.getByRole("button", { name: "Existing source" }))
    expect(window.location.search).toContain("wikiSource=exists")
  })
})
