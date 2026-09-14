import { act, render, screen, waitFor, cleanup } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { WikiDataProvider, useWikiData, useWikiQuery } from "./wiki-data"
const backend = vi.hoisted(() => ({
  call: vi.fn(),
  events: new Map<string, () => void>(),
  reconnect: undefined as (() => void) | undefined,
}))
vi.mock("@/lib/transport", () => ({
  getTransport: () => ({
    call: backend.call,
    subscribe: async (event: string, callback: () => void) => {
      backend.events.set(event, callback)
      return () => backend.events.delete(event)
    },
    onReconnect: (fn: () => void) => {
      backend.reconnect = fn
      return () => {
        backend.reconnect = undefined
      }
    },
  }),
}))
function Query({ path }: { path: string }) {
  const { data, error } = useWikiQuery<{ body: string }>("wiki_read_note", {
    path,
  })
  return <div>{data?.body || error || "Waiting"}</div>
}
function Route() {
  const { route, navigate } = useWikiData()
  return (
    <button onClick={() => navigate({ path: "work/a.md" })}>
      {route.path || route.view}
    </button>
  )
}
beforeEach(() => {
  backend.call.mockReset()
  backend.events.clear()
  window.history.replaceState({}, "", "/")
})
afterEach(() => {
  cleanup()
  vi.useRealTimers()
})
describe("shared Wiki data refresh", () => {
  it("ignores obsolete note responses and refreshes the same path after content events", async () => {
    let staleResolve: (value: { body: string }) => void = () => {}
    let content = "B first"
    backend.call.mockImplementation(
      (command: string, args?: { path: string }) =>
        command === "wiki_get_overview"
          ? Promise.resolve({ active_job_count: 0 })
          : args?.path === "a.md"
            ? new Promise((resolve) => {
                staleResolve = resolve
              })
            : Promise.resolve({ body: content })
    )
    const { rerender } = render(
      <WikiDataProvider>
        <Query path="a.md" />
      </WikiDataProvider>
    )
    rerender(
      <WikiDataProvider>
        <Query path="b.md" />
      </WikiDataProvider>
    )
    expect(await screen.findByText("B first")).toBeVisible()
    await act(async () => staleResolve({ body: "Stale A" }))
    expect(screen.queryByText("Stale A")).not.toBeInTheDocument()
    content = "B updated"
    act(() => backend.events.get("wiki://content-changed")?.())
    expect(await screen.findByText("B updated")).toBeVisible()
    content = "B reconnected"
    act(() => backend.reconnect?.())
    expect(await screen.findByText("B reconnected")).toBeVisible()
  })
  it("polls active work only while visible, then rereads after focus", async () => {
    vi.useFakeTimers()
    let content = "initial"
    backend.call.mockImplementation((command: string) =>
      Promise.resolve(
        command === "wiki_get_overview"
          ? { active_job_count: 1 }
          : { body: content }
      )
    )
    render(
      <WikiDataProvider>
        <Query path="a.md" />
      </WikiDataProvider>
    )
    await act(async () => {
      await Promise.resolve()
    })
    content = "poll update"
    await act(async () => vi.advanceTimersByTimeAsync(5000))
    expect(screen.getByText("poll update")).toBeVisible()
    const visibility = vi
      .spyOn(document, "visibilityState", "get")
      .mockReturnValue("hidden")
    content = "hidden update"
    await act(async () => vi.advanceTimersByTimeAsync(5000))
    expect(screen.queryByText("hidden update")).not.toBeInTheDocument()
    visibility.mockRestore()
    await act(async () => window.dispatchEvent(new Event("focus")))
    expect(screen.getByText("hidden update")).toBeVisible()
  })
  it("restores static query navigation on browser history changes", async () => {
    window.history.replaceState({}, "", "/?wikiView=overview")
    backend.call.mockResolvedValue({ active_job_count: 0 })
    render(
      <WikiDataProvider>
        <Route />
      </WikiDataProvider>
    )
    act(() => screen.getByRole("button").click())
    expect(window.location.search).toContain("wikiPath=work%2Fa.md")
    expect(window.location.search).toContain("wikiView=work")
    window.history.replaceState(
      {},
      "",
      "/?wikiView=sources&wikiSource=source-a"
    )
    act(() => window.dispatchEvent(new PopStateEvent("popstate")))
    await waitFor(() =>
      expect(screen.getByRole("button")).toHaveTextContent("sources")
    )
  })
  it("keeps note navigation in the default library", async () => {
    backend.call.mockResolvedValue({ active_job_count: 0 })
    await act(async () => {
      render(
        <WikiDataProvider>
          <Route />
        </WikiDataProvider>
      )
    })
    expect(screen.getByRole("button")).toHaveTextContent("library")
    act(() => screen.getByRole("button").click())
    expect(window.location.search).toContain("wikiView=library")
    expect(window.location.search).toContain("wikiPath=work%2Fa.md")
  })
})
