import { cleanup, render, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
const mocks = vi.hoisted(() => ({ replace: vi.fn(), desktop: vi.fn() }))
vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: mocks.replace }),
}))
vi.mock("@/lib/platform", () => ({ isDesktop: mocks.desktop }))
import Page from "./page"

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
  vi.clearAllMocks()
  localStorage.removeItem("codeg_token")
  window.history.replaceState(null, "", "/")
})
describe("workspace reading links", () => {
  it("preserves Wiki query and anchor when entering desktop workspace", () => {
    mocks.desktop.mockReturnValue(true)
    window.history.replaceState(
      null,
      "",
      "/?wikiView=work&wikiPath=work%2Frecords%2Fa.md#checks"
    )
    render(<Page />)
    expect(mocks.replace).toHaveBeenCalledWith(
      "/workspace?wikiView=work&wikiPath=work%2Frecords%2Fa.md#checks"
    )
  })
  it("preserves Wiki links after web token validation", async () => {
    mocks.desktop.mockReturnValue(false)
    localStorage.setItem("codeg_token", "test-token")
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true }))
    window.history.replaceState(
      null,
      "",
      "/?wikiView=sources&wikiSource=example"
    )
    render(<Page />)
    await waitFor(() =>
      expect(mocks.replace).toHaveBeenCalledWith(
        "/workspace?wikiView=sources&wikiSource=example"
      )
    )
  })
})
