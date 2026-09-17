import { render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { beforeEach, describe, expect, it, vi } from "vitest"

const mocks = vi.hoisted(() => ({
  isLocalDesktop: vi.fn(() => true),
  openPath: vi.fn(async () => {}),
  revealItemInDir: vi.fn(async () => {}),
}))

vi.mock("next-intl", () => ({
  useTranslations: () => (key: string) => key,
}))

vi.mock("@/lib/platform", () => ({
  isLocalDesktop: mocks.isLocalDesktop,
  openPath: mocks.openPath,
  revealItemInDir: mocks.revealItemInDir,
}))

import { FileOpenDropdown } from "./file-open-dropdown"

const PATH = "/Users/me/docs/手册.docx"

beforeEach(() => {
  vi.clearAllMocks()
  mocks.isLocalDesktop.mockReturnValue(true)
})

describe("FileOpenDropdown", () => {
  it("hides on web / remote-desktop where the OS openers no-op", () => {
    mocks.isLocalDesktop.mockReturnValue(false)
    const { container } = render(<FileOpenDropdown path={PATH} />)
    expect(container).toBeEmptyDOMElement()
  })

  it("opens the file with the default app", async () => {
    const user = userEvent.setup()
    render(<FileOpenDropdown path={PATH} />)
    await user.click(screen.getByRole("button", { name: "open" }))
    await user.click(await screen.findByRole("menuitem", { name: "defaultApp" }))
    expect(mocks.openPath).toHaveBeenCalledWith(PATH)
    expect(mocks.revealItemInDir).not.toHaveBeenCalled()
  })

  it("reveals the file in the folder", async () => {
    const user = userEvent.setup()
    render(<FileOpenDropdown path={PATH} />)
    await user.click(screen.getByRole("button", { name: "open" }))
    await user.click(await screen.findByRole("menuitem", { name: "folder" }))
    expect(mocks.revealItemInDir).toHaveBeenCalledWith(PATH)
    expect(mocks.openPath).not.toHaveBeenCalled()
  })
})
