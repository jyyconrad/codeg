import { act, render, screen, fireEvent, cleanup } from "@testing-library/react"
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
afterEach(() => {
  cleanup()
  vi.useRealTimers()
})
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

it("searches after typing settles and submits immediately with Enter", async () => {
  vi.useFakeTimers()
  window.history.replaceState({}, "", "/?wikiView=overview")
  render(
    <NextIntlClientProvider locale="en" messages={messages}>
      <WikiPage />
    </NextIntlClientProvider>
  )
  fireEvent.change(
    screen.getByRole("textbox", { name: messages.Wiki.v2.search }),
    { target: { value: "retry" } }
  )
  await act(async () => vi.advanceTimersByTimeAsync(299))
  expect(window.location.search).not.toContain("wikiQuery=retry")
  await act(async () => vi.advanceTimersByTimeAsync(1))
  expect(window.location.search).toContain("wikiQuery=retry")
  fireEvent.change(
    screen.getByRole("textbox", { name: messages.Wiki.v2.search }),
    { target: { value: "new" } }
  )
  fireEvent.submit(
    screen
      .getByRole("textbox", { name: messages.Wiki.v2.search })
      .closest("form")!
  )
  expect(window.location.search).toContain("wikiQuery=new")
})
