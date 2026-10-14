import { act, render, screen, cleanup } from "@testing-library/react"
import { NextIntlClientProvider } from "next-intl"
import { afterEach, expect, it, vi } from "vitest"
import messages from "@/i18n/messages/en.json"
import { WikiPage } from "./wiki-page"
vi.mock("@/lib/transport", () => ({
  getTransport: () => ({
    call: async () => ({ active_job_count: 0 }),
    subscribe: async () => () => {},
  }),
}))
vi.mock("./wiki-all-view", () => ({ WikiAllView: () => null }))
vi.mock("./wiki-import-dialog", () => ({ WikiImportButton: () => null }))
vi.mock("./wiki-work-view", () => ({ WikiWorkView: () => null }))
vi.mock("./wiki-capabilities-view", () => ({
  WikiCapabilitiesView: () => null,
}))
vi.mock("./wiki-sources-view", () => ({ WikiSourcesView: () => null }))
vi.mock("./wiki-jobs-view", () => ({ WikiJobsView: () => null }))
vi.mock("./wiki-shared", () => ({ WikiNoteBrowser: () => null }))
afterEach(cleanup)
it("does not render a settings link that would replace the workspace window", async () => {
  window.history.replaceState({}, "", "/?wikiView=overview")
  await act(async () => {
    render(
      <NextIntlClientProvider locale="en" messages={messages}>
        <WikiPage />
      </NextIntlClientProvider>
    )
  })
  expect(
    screen.queryByRole("link", { name: messages.Wiki.v2.settings })
  ).toBeNull()
})

it("does not rewrite a legacy overview URL into domain search", async () => {
  window.history.replaceState({}, "", "/?wikiView=overview")
  await act(async () => {
    render(
      <NextIntlClientProvider locale="en" messages={messages}>
        <WikiPage />
      </NextIntlClientProvider>
    )
  })
  expect(
    screen.getByRole("button", { name: messages.Wiki.v2.nav.files })
  ).toHaveAttribute("aria-current", "page")
  expect(
    screen.queryByRole("textbox", { name: messages.Wiki.v2.search })
  ).not.toBeInTheDocument()
  expect(window.location.search).not.toContain("wikiQuery=")
  expect(window.location.search).not.toContain("wikiView=sources")
})
