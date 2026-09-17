import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react"
import { NextIntlClientProvider } from "next-intl"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import messages from "@/i18n/messages/en.json"
import { WikiPage } from "./wiki-page"

const backend = vi.hoisted(() => ({
  call: vi.fn(),
  copy: vi.fn(),
  events: new Map<string, () => void>(),
}))
vi.mock("@/lib/transport", () => ({
  getTransport: () => ({
    call: backend.call,
    subscribe: async (event: string, callback: () => void) => {
      backend.events.set(event, callback)
      return () => backend.events.delete(event)
    },
  }),
}))
vi.mock("@/lib/platform", () => ({ isLocalDesktop: () => false }))
vi.mock("@/lib/utils", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/utils")>()),
  copyTextToClipboard: backend.copy,
}))
vi.mock("./wiki-all-view", () => ({ WikiAllView: () => <p>Overview</p> }))
vi.mock("./wiki-import-dialog", () => ({ WikiImportButton: () => null }))
vi.mock("./wiki-work-view", () => ({ WikiWorkView: () => <p>Work list</p> }))
vi.mock("./wiki-capabilities-view", () => ({
  WikiCapabilitiesView: () => null,
}))
vi.mock("./wiki-sources-view", () => ({ WikiSourcesView: () => null }))
vi.mock("./wiki-jobs-view", () => ({ WikiJobsView: () => null }))
vi.mock("@/components/ai-elements/markdown-link", () => ({
  MarkdownLink: ({
    href,
    children,
  }: {
    href: string
    children: React.ReactNode
  }) => <a href={href}>{children}</a>,
}))

const library = {
  vault_path: "/server/personal-wiki",
  home_path: "index.md",
  warnings: [],
  tree: [
    { path: "index.md", name: "index.md", title: "Home", is_dir: false },
    {
      path: "work",
      name: "work",
      is_dir: true,
      children: [
        {
          path: "work/index.md",
          name: "index.md",
          title: "Work",
          is_dir: false,
        },
        {
          path: "work/records",
          name: "records",
          is_dir: true,
          children: [
            {
              path: "work/records/a.md",
              name: "a.md",
              title: "Article A",
              is_dir: false,
            },
            {
              path: "work/records/b.md",
              name: "b.md",
              title: "Article B",
              is_dir: false,
            },
          ],
        },
      ],
    },
  ],
}
let contents: Record<string, string>
function document(path: string) {
  return {
    note: {
      note_id: path,
      path,
      title:
        path === "index.md"
          ? "Wiki home"
          : path === "work/index.md"
            ? "Work"
            : path.endsWith("a.md")
              ? "Article A"
              : "Article B",
      type:
        path === "index.md" || path.endsWith("/index.md") ? "index" : "note",
      summary: "",
      updated_at: null,
      project_ids: [],
      source_ids: path.endsWith("a.md") ? ["src"] : [],
      evidence_level: null,
    },
    body: contents[path],
    source: contents[path],
    sources: path.endsWith("a.md")
      ? [
          {
            title: "Pagination turn",
            path: "raw/sessions/src.md",
            source_id: "src",
            availability: "available",
            start_line: 1,
            end_line: 4,
            excerpt: "secret dump",
            source_url: null,
          },
        ]
      : [],
    headings: [],
    format_warning: false,
  }
}
function mount() {
  return render(
    <NextIntlClientProvider locale="en" messages={messages}>
      <WikiPage />
    </NextIntlClientProvider>
  )
}
beforeEach(() => {
  window.history.replaceState({}, "", "/")
  backend.call.mockReset()
  backend.copy.mockReset().mockResolvedValue(true)
  backend.events.clear()
  contents = {
    "index.md": "[Start reading](work/records/a.md)",
    "work/index.md": "Work landing page",
    "work/records/a.md": "[Next article](b.md)",
    "work/records/b.md": "Current B content",
  }
  backend.call.mockImplementation(
    async (command: string, args?: { path: string }) => {
      if (command === "wiki_get_overview") return { active_job_count: 0 }
      if (command === "wiki_refresh_library") return library
      if (command === "wiki_read_note" && args) {
        if (!(args.path in contents)) throw new Error("Missing")
        return document(args.path)
      }
      throw new Error(command)
    }
  )
})
afterEach(cleanup)

describe("Wiki library reading", () => {
  it("opens a real Markdown home and shows the Obsidian folder location", async () => {
    mount()
    expect(
      await screen.findByRole("heading", { name: "Wiki home" })
    ).toBeVisible()
    expect(
      screen.getByRole("navigation", {
        name: messages.Wiki.v2.library.directory,
      })
    ).toBeInTheDocument()
    expect(screen.getByText(library.vault_path)).toBeVisible()
    expect(screen.getByText(messages.Wiki.v2.library.serverHint)).toBeVisible()
    fireEvent.click(
      screen.getByRole("button", { name: messages.Wiki.v2.library.copyPath })
    )
    expect(backend.copy).toHaveBeenCalledWith(library.vault_path)
    expect(
      await screen.findByText(messages.Wiki.v2.library.copied)
    ).toBeVisible()
  })

  it("opens a folder index when the folder name is clicked, while the chevron only expands", async () => {
    mount()
    expect(
      await screen.findByRole("heading", { name: "Wiki home" })
    ).toBeVisible()
    const directory = screen.getByRole("navigation", {
      name: messages.Wiki.v2.library.directory,
    })
    fireEvent.click(within(directory).getByRole("button", { name: "work" }))
    expect(await screen.findByRole("heading", { name: "Work" })).toBeVisible()
    expect(window.location.search).toContain("wikiPath=work%2Findex.md")
    expect(
      within(directory).getByRole("button", { name: "work" })
    ).toHaveAttribute("aria-expanded", "true")
    fireEvent.click(
      within(directory).getByRole("button", {
        name: messages.Wiki.v2.library.expandDirectory.replace(
          "{name}",
          "records"
        ),
      })
    )
    expect(await screen.findByRole("heading", { name: "Work" })).toBeVisible()
    expect(
      within(directory).getByRole("button", { name: "records" })
    ).toHaveAttribute("aria-expanded", "true")
  })

  it("follows links and directory selection without leaving the library, then restores history", async () => {
    mount()
    fireEvent.click(await screen.findByRole("link", { name: "Start reading" }))
    expect(
      await screen.findByRole("heading", { name: "Article A" })
    ).toBeVisible()
    expect(window.location.search).toContain("wikiView=library")
    expect(screen.getByRole("button", { name: "records" })).toHaveAttribute(
      "aria-expanded",
      "true"
    )
    fireEvent.click(screen.getByRole("link", { name: "Next article" }))
    expect(
      await screen.findByRole("heading", { name: "Article B" })
    ).toBeVisible()
    expect(window.location.search).toContain("wikiPath=work%2Frecords%2Fb.md")
    fireEvent.click(screen.getByRole("button", { name: "Article A" }))
    expect(
      await screen.findByRole("heading", { name: "Article A" })
    ).toBeVisible()
    window.history.replaceState(
      {},
      "",
      "/?wikiView=library&wikiPath=work%2Frecords%2Fb.md"
    )
    act(() => window.dispatchEvent(new PopStateEvent("popstate")))
    expect(
      await screen.findByRole("heading", { name: "Article B" })
    ).toBeVisible()
    expect(screen.getByRole("button", { name: "Article B" })).toHaveAttribute(
      "aria-current",
      "page"
    )
    fireEvent.click(
      screen.getByRole("button", { name: messages.Wiki.v2.library.backHome })
    )
    expect(
      await screen.findByRole("heading", { name: "Wiki home" })
    ).toBeVisible()
  })

  it("refreshes the library before rereading the current page and retains selection", async () => {
    window.history.replaceState(
      {},
      "",
      "/?wikiView=library&wikiPath=work%2Frecords%2Fb.md"
    )
    mount()
    expect(await screen.findByText("Current B content")).toBeVisible()
    const initialReadCount = backend.call.mock.calls.filter(
      ([command]) => command === "wiki_read_note"
    ).length
    let finish: (value: typeof library) => void = () => {}
    backend.call.mockImplementation(
      async (command: string, args?: { path: string }) => {
        if (command === "wiki_get_overview") return { active_job_count: 0 }
        if (command === "wiki_refresh_library")
          return new Promise<typeof library>((resolve) => {
            finish = resolve
          })
        return document(args!.path)
      }
    )
    act(() => backend.events.get("wiki://content-changed")?.())
    await waitFor(() =>
      expect(backend.call).toHaveBeenCalledWith("wiki_refresh_library", {})
    )
    expect(
      backend.call.mock.calls.filter(
        ([command]) => command === "wiki_read_note"
      )
    ).toHaveLength(initialReadCount)
    contents["work/records/b.md"] = "Refreshed B content"
    await act(async () => finish(library))
    expect(await screen.findByText("Refreshed B content")).toBeVisible()
    expect(screen.getByRole("button", { name: "Article B" })).toHaveAttribute(
      "aria-current",
      "page"
    )
  })

  it("shows material titles instead of dump paths on a note", async () => {
    window.history.replaceState(
      {},
      "",
      "/?wikiView=library&wikiPath=work%2Frecords%2Fa.md"
    )
    mount()
    expect(
      await screen.findByRole("heading", { name: "Article A" })
    ).toBeVisible()
    expect(screen.getByText("Pagination turn")).toBeVisible()
    expect(screen.queryByText(/raw\/sessions/)).not.toBeInTheDocument()
    expect(screen.queryByText("secret dump")).not.toBeInTheDocument()
  })

  it("searches from the library using the existing overview results", async () => {
    mount()
    await screen.findByRole("heading", { name: "Wiki home" })
    const search = screen.getByRole("textbox", {
      name: messages.Wiki.v2.search,
    })
    fireEvent.change(search, { target: { value: "repository" } })
    fireEvent.submit(search.closest("form")!)
    expect(window.location.search).toContain("wikiView=overview")
    expect(window.location.search).toContain("wikiQuery=repository")
    expect(
      screen.queryByRole("heading", { name: "Wiki home" })
    ).not.toBeInTheDocument()
  })
})
