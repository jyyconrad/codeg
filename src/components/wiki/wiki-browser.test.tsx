import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { NextIntlClientProvider } from "next-intl"
import { afterEach, expect, it, vi } from "vitest"
import messages from "@/i18n/messages/en.json"
import { WikiDataProvider } from "./wiki-data"
import { WikiNoteBrowser } from "./wiki-shared"

const backend = vi.hoisted(() => ({ call: vi.fn() }))
vi.mock("@/lib/transport", () => ({
  getTransport: () => ({
    call: backend.call,
    subscribe: async () => () => {},
  }),
}))
vi.mock("@/components/ai-elements/markdown-link", () => ({
  MarkdownLink: ({
    href,
    children,
  }: {
    href: string
    children: React.ReactNode
  }) => <a href={href}>{children}</a>,
}))
afterEach(cleanup)

it("opens a legacy file URL, then records the next file on wikiPath", async () => {
  window.history.replaceState(
    {},
    "",
    "/?wikiView=capabilities&wikiPath=raw%2Fa.md"
  )
  let secondContent = "## Second original"
  const files = [
    { path: "raw/a.md", name: "a.md", is_dir: false },
    { path: "raw/b.md", name: "b.md", is_dir: false },
  ]
  const kept = [
    { path: "journal", name: "journal", is_dir: true },
    { path: "sources", name: "sources", is_dir: true },
    { path: "capabilities", name: "capabilities", is_dir: true },
    { path: "knowledge", name: "knowledge", is_dir: true },
    { path: "work", name: "work", is_dir: true },
    { path: "AGENTS.md", name: "AGENTS.md", is_dir: false },
    { path: "log.md", name: "log.md", is_dir: false },
  ]
  backend.call.mockImplementation(
    async (command: string, args?: { path?: string }) => {
      if (command === "wiki_get_overview") return { active_job_count: 0 }
      if (command === "wiki_vault_tree") {
        return args?.path === "raw"
          ? [...files]
          : [{ path: "raw", name: "raw", is_dir: true }, ...kept]
      }
      if (command === "wiki_vault_read") {
        return {
          path: args?.path,
          content:
            args?.path === "raw/a.md" ? "## First original" : secondContent,
        }
      }
      throw new Error(command)
    }
  )
  render(
    <NextIntlClientProvider locale="en" messages={messages}>
      <WikiDataProvider>
        <WikiNoteBrowser
          emptyTitle="Internal files"
          emptyDescription="Browse original Markdown"
          includeRaw
        />
      </WikiDataProvider>
    </NextIntlClientProvider>
  )
  expect(
    await screen.findByRole("heading", { name: "First original" })
  ).toBeVisible()
  for (const name of [
    "journal",
    "sources",
    "capabilities",
    "knowledge",
    "work",
    "AGENTS.md",
    "log.md",
  ])
    expect(screen.getByRole("button", { name })).toBeVisible()
  fireEvent.click(screen.getByRole("button", { name: "raw" }))
  expect(screen.getByRole("button", { name: "raw" })).toHaveAttribute(
    "aria-expanded",
    "true"
  )
  expect(backend.call).toHaveBeenCalledWith("wiki_vault_tree", {
    path: "raw",
    recursive: true,
    include_raw: true,
  })
  expect(backend.call).toHaveBeenCalledWith("wiki_vault_read", {
    path: "raw/a.md",
  })
  fireEvent.click(await screen.findByRole("button", { name: "b.md" }))
  expect(
    await screen.findByRole("heading", { name: "Second original" })
  ).toBeVisible()
  expect(
    screen.queryByRole("heading", { name: "First original" })
  ).not.toBeInTheDocument()
  expect(window.location.search).toContain("wikiView=files")
  expect(window.location.search).toContain("wikiPath=raw%2Fb.md")
  expect(window.location.search).not.toContain("wikiView=capabilities")
  secondContent = "## Externally updated original"
  files.push({ path: "raw/c.md", name: "c.md", is_dir: false })
  fireEvent.click(screen.getByRole("button", { name: messages.Wiki.refresh }))
  expect(
    await screen.findByRole("heading", { name: "Externally updated original" })
  ).toBeVisible()
  expect(screen.getByRole("button", { name: "b.md" })).toHaveAttribute(
    "aria-current",
    "page"
  )
  expect(await screen.findByRole("button", { name: "c.md" })).toBeVisible()
})
