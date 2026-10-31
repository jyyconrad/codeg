import { cleanup, render, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"

const mocks = vi.hoisted(() => ({ replace: vi.fn(), desktop: vi.fn() }))
vi.mock("next/navigation", () => ({
  useRouter: () => ({ replace: mocks.replace }),
}))
vi.mock("@/lib/platform", () => ({ isDesktop: mocks.desktop }))

import Page from "./page"

const reply = (status: number, challenge?: string) => ({
  ok: status >= 200 && status < 300,
  status,
  headers: new Headers(challenge ? { "WWW-Authenticate": challenge } : {}),
})

let fetchMock: ReturnType<typeof vi.fn>

beforeEach(() => {
  mocks.replace.mockClear()
  mocks.desktop.mockReturnValue(false)
  localStorage.setItem("codeg_token", "tok")
  fetchMock = vi.fn()
  vi.stubGlobal("fetch", fetchMock)
})

afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
  vi.clearAllMocks()
  localStorage.clear()
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
    fetchMock.mockResolvedValue(reply(200))
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

describe("root page token check", () => {
  it("asks a proxy for a 401 instead of a redirect", async () => {
    fetchMock.mockResolvedValue(reply(200))
    render(<Page />)
    await waitFor(() => expect(mocks.replace).toHaveBeenCalled())
    expect(fetchMock.mock.calls[0][1].headers).toMatchObject({
      Authorization: "Bearer tok",
      "X-Requested-With": "XMLHttpRequest",
    })
  })

  it("clears the token and goes to /login when Codeg rejects it", async () => {
    fetchMock.mockResolvedValue(reply(401, 'Bearer realm="codeg"'))
    render(<Page />)
    await waitFor(() => expect(mocks.replace).toHaveBeenCalledWith("/login"))
    expect(localStorage.getItem("codeg_token")).toBeNull()
  })

  it("keeps the token when the 401 came from a proxy in front of Codeg", async () => {
    fetchMock.mockResolvedValue(
      reply(401, 'Cloudflare-Access resource_metadata="https://x.test/m"')
    )
    render(<Page />)
    // The workspace's connection guard offers the reload from there.
    await waitFor(() =>
      expect(mocks.replace).toHaveBeenCalledWith("/workspace")
    )
    expect(localStorage.getItem("codeg_token")).toBe("tok")
  })
})

describe("root page without a token", () => {
  beforeEach(() => localStorage.removeItem("codeg_token"))

  it("enters the workspace when a proxy in front of Codeg admits the browser", async () => {
    fetchMock.mockResolvedValue(reply(200))
    render(<Page />)
    await waitFor(() =>
      expect(mocks.replace).toHaveBeenCalledWith("/workspace")
    )
    expect(localStorage.getItem("codeg_proxy_auth")).toBe("1")
  })

  it("asks for the token when the server wants one", async () => {
    fetchMock.mockResolvedValue(reply(401, 'Bearer realm="codeg"'))
    render(<Page />)
    await waitFor(() => expect(mocks.replace).toHaveBeenCalledWith("/login"))
  })
})
