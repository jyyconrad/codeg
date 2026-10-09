import { render, screen } from "@testing-library/react"
import { NextIntlClientProvider } from "next-intl"
import { beforeEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/api", () => ({
  getChatMessageLanguage: vi.fn(),
  setChatMessageLanguage: vi.fn(),
}))

vi.mock("sonner", () => ({
  toast: {
    success: vi.fn(),
    error: vi.fn(),
  },
}))

import { ChannelOtherTab } from "./channel-other-tab"
import enMessages from "@/i18n/messages/en.json"
import { getChatMessageLanguage } from "@/lib/api"

const mockGetLanguage = vi.mocked(getChatMessageLanguage)

function renderTab() {
  return render(
    <NextIntlClientProvider locale="en" messages={enMessages}>
      <ChannelOtherTab />
    </NextIntlClientProvider>
  )
}

beforeEach(() => {
  vi.clearAllMocks()
  mockGetLanguage.mockResolvedValue("en")
})

describe("ChannelOtherTab inbound routing", () => {
  it("explains that an unquoted message starts a new conversation", async () => {
    renderTab()
    expect(
      await screen.findByRole("heading", { name: "Inbound messages" })
    ).toBeInTheDocument()
    expect(screen.getByText(/default agent/i)).toBeInTheDocument()
    expect(screen.queryByRole("spinbutton")).not.toBeInTheDocument()
  })
})
