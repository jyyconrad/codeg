import { beforeEach, describe, expect, it, vi } from "vitest"

const mocks = vi.hoisted(() => ({
  pathExists: vi.fn(),
  expandHomePath: vi.fn(async (p: string) => p.replace(/^~/, "/Users/me")),
}))

vi.mock("@/lib/api", () => ({
  pathExists: mocks.pathExists,
}))

vi.mock("@/lib/file-open-target", async () => {
  const actual = await vi.importActual<typeof import("@/lib/file-open-target")>(
    "@/lib/file-open-target"
  )
  return {
    ...actual,
    expandHomePath: mocks.expandHomePath,
  }
})

import { ensureLocalFileExists } from "./local-file-exists"

beforeEach(() => {
  vi.clearAllMocks()
  mocks.pathExists.mockResolvedValue(true)
})

describe("ensureLocalFileExists", () => {
  it("returns not-found when the resolved path is not a file", async () => {
    mocks.pathExists.mockResolvedValue(false)
    const result = await ensureLocalFileExists(
      "docs/使用手册.docx",
      "/Users/me/repo"
    )
    expect(result).toEqual({ ok: false, reason: "not-found" })
    expect(mocks.pathExists).toHaveBeenCalledWith(
      "/Users/me/repo/docs/使用手册.docx"
    )
  })

  it("joins a relative path onto the workspace folder before checking", async () => {
    const result = await ensureLocalFileExists("docs/a.docx", "/repo")
    expect(result).toEqual({ ok: true, path: "/repo/docs/a.docx" })
  })

  it("returns no-workspace when a relative path has no folder to join", async () => {
    const result = await ensureLocalFileExists("docs/a.docx", null)
    expect(result).toEqual({ ok: false, reason: "no-workspace" })
    expect(mocks.pathExists).not.toHaveBeenCalled()
  })
})
