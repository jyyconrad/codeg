import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import { NextIntlClientProvider } from "next-intl"
import { beforeEach, describe, expect, it, vi } from "vitest"

vi.mock("@/lib/api", () => ({
  getChatMessageLanguage: vi.fn(),
  setChatMessageLanguage: vi.fn(),
  getChatFolderInboundIdleMinutes: vi.fn(),
  setChatFolderInboundIdleMinutes: vi.fn(),
}))

vi.mock("sonner", () => ({
  toast: {
    success: vi.fn(),
    error: vi.fn(),
  },
}))

import { ChannelOtherTab, parseIdleMinutesInput } from "./channel-other-tab"
import enMessages from "@/i18n/messages/en.json"
import {
  getChatFolderInboundIdleMinutes,
  getChatMessageLanguage,
  setChatFolderInboundIdleMinutes,
} from "@/lib/api"
import { toast } from "sonner"

const mockGetLanguage = vi.mocked(getChatMessageLanguage)
const mockGetIdle = vi.mocked(getChatFolderInboundIdleMinutes)
const mockSetIdle = vi.mocked(setChatFolderInboundIdleMinutes)

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
  mockGetIdle.mockResolvedValue(30)
  mockSetIdle.mockResolvedValue(undefined)
})

describe("parseIdleMinutesInput", () => {
  it("accepts the inclusive 0..10080 range and rejects the rest", () => {
    expect(parseIdleMinutesInput("0")).toBe(0)
    expect(parseIdleMinutesInput("30")).toBe(30)
    expect(parseIdleMinutesInput("10080")).toBe(10080)
    expect(parseIdleMinutesInput(" 5 ")).toBe(5)
    expect(parseIdleMinutesInput("-1")).toBeNull()
    expect(parseIdleMinutesInput("10081")).toBeNull()
    expect(parseIdleMinutesInput("1.5")).toBeNull()
    expect(parseIdleMinutesInput("abc")).toBeNull()
    expect(parseIdleMinutesInput("")).toBeNull()
  })
})

describe("ChannelOtherTab folder inbound idle", () => {
  it("loads the stored idle minutes into the input", async () => {
    mockGetIdle.mockResolvedValue(15)
    renderTab()
    const input = await screen.findByRole("spinbutton", {
      name: "Folder session reuse",
    })
    expect(input).toHaveValue(15)
  })

  it("saves a whole number in range", async () => {
    renderTab()
    const input = await screen.findByRole("spinbutton", {
      name: "Folder session reuse",
    })
    fireEvent.change(input, { target: { value: "5" } })
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() => expect(mockSetIdle).toHaveBeenCalledWith(5))
    expect(toast.success).toHaveBeenCalled()
  })

  it("does not save an out-of-range value", async () => {
    renderTab()
    const input = await screen.findByRole("spinbutton", {
      name: "Folder session reuse",
    })
    fireEvent.change(input, { target: { value: "10081" } })
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() => expect(toast.error).toHaveBeenCalled())
    expect(mockSetIdle).not.toHaveBeenCalled()
  })
})
