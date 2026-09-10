import { render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { NextIntlClientProvider } from "next-intl"
import { beforeEach, describe, expect, it } from "vitest"
import enMessages from "@/i18n/messages/en.json"
import { ToolboxView } from "./toolbox-view"
import { useToolboxStore } from "./toolbox-store"

function renderView() {
  return render(
    <NextIntlClientProvider locale="en" messages={enMessages}>
      <ToolboxView />
    </NextIntlClientProvider>
  )
}

describe("ToolboxView", () => {
  beforeEach(() => {
    localStorage.clear()
    useToolboxStore.setState({
      selectedToolId: null,
      favorites: [],
      recent: [],
      pendingInput: null,
      hydrated: false,
    })
  })

  it("lists v1 categories and filters by alias", async () => {
    const user = userEvent.setup()
    renderView()
    expect(screen.getByText("Encrypt / Decrypt")).toBeTruthy()
    expect(
      screen.getByRole("button", { name: "AES / SM4 encrypt" })
    ).toBeTruthy()

    await user.type(screen.getByLabelText("Search tools"), "md5加密")
    expect(screen.getByRole("button", { name: "Hash & checksum" })).toBeTruthy()
    expect(
      screen.queryByRole("button", { name: "JSON format & validate" })
    ).toBeNull()
  })

  it("opens a tool from the catalog", async () => {
    const user = userEvent.setup()
    renderView()
    await user.click(
      screen.getByRole("button", { name: "Base64 encode / decode" })
    )
    await waitFor(() => {
      expect(screen.getByText("This is encoding, not encryption.")).toBeTruthy()
    })
  })
})
