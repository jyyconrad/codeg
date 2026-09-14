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

it("loads an internal directory, switches documents and refreshes external changes without changing the main route", async () => {
  window.history.replaceState({}, "", "/?wikiView=library")
  let secondContent = "## Second original"
  const files = [
    { path: "raw/a.md", name: "a.md", is_dir: false },
    { path: "raw/b.md", name: "b.md", is_dir: false },
  ]
  backend.call.mockImplementation(
    async (command: string, args?: { path?: string }) => {
      if (command === "wiki_get_overview") return { active_job_count: 0 }
      if (command === "wiki_vault_tree") {
        return args?.path === "raw"
          ? [...files]
          : [{ path: "raw", name: "raw", is_dir: true }]
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
  fireEvent.click(await screen.findByRole("button", { name: "raw" }))
  expect(screen.getByRole("button", { name: "raw" })).toHaveAttribute(
    "aria-expanded",
    "true"
  )
  fireEvent.click(await screen.findByRole("button", { name: "a.md" }))
  expect(
    await screen.findByRole("heading", { name: "First original" })
  ).toBeVisible()
  expect(backend.call).toHaveBeenCalledWith("wiki_vault_tree", {
    path: "raw",
    recursive: true,
    include_raw: true,
  })
  fireEvent.click(screen.getByRole("button", { name: "b.md" }))
  expect(
    await screen.findByRole("heading", { name: "Second original" })
  ).toBeVisible()
  expect(
    screen.queryByRole("heading", { name: "First original" })
  ).not.toBeInTheDocument()
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
  expect(window.location.search).toBe("?wikiView=library")
})
