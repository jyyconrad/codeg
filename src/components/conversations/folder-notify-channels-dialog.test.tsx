import { render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { NextIntlClientProvider } from "next-intl"
import { beforeEach, describe, expect, it, vi } from "vitest"

import enMessages from "@/i18n/messages/en.json"
import type { ChatChannelInfo } from "@/lib/types"

const api = vi.hoisted(() => ({
  listChatChannels: vi.fn(),
  listFolderChatChannels: vi.fn(),
  setFolderChatChannels: vi.fn(),
}))

vi.mock("@/lib/api", () => api)

vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn() },
}))

import { FolderNotifyChannelsDialog } from "./folder-notify-channels-dialog"

function channel(id: number, name: string, enabled = true): ChatChannelInfo {
  return {
    id,
    name,
    channel_type: "telegram",
    enabled,
    config_json: "{}",
    event_filter_json: null,
    daily_report_enabled: false,
    daily_report_time: null,
    created_at: "2026-01-01T00:00:00.000Z",
    updated_at: "2026-01-01T00:00:00.000Z",
  }
}

function renderDialog() {
  return render(
    <NextIntlClientProvider locale="en" messages={enMessages}>
      <FolderNotifyChannelsDialog folderId={1} open onOpenChange={() => {}} />
    </NextIntlClientProvider>
  )
}

describe("FolderNotifyChannelsDialog", () => {
  beforeEach(() => {
    api.listChatChannels.mockReset()
    api.listFolderChatChannels.mockReset()
    api.setFolderChatChannels.mockReset()
    api.listChatChannels.mockResolvedValue([
      channel(2, "Telegram"),
      channel(3, "Lark"),
    ])
    api.listFolderChatChannels.mockResolvedValue([])
    api.setFolderChatChannels.mockResolvedValue([2, 3])
  })

  it("saves the checked channel ids", async () => {
    const user = userEvent.setup()
    renderDialog()

    await user.click(await screen.findByRole("checkbox", { name: "Telegram" }))
    await user.click(screen.getByRole("checkbox", { name: "Lark" }))
    await user.click(screen.getByRole("button", { name: /save/i }))

    await waitFor(() =>
      expect(api.setFolderChatChannels).toHaveBeenCalledWith(
        1,
        expect.arrayContaining([2, 3])
      )
    )
  })

  it("keeps disabled channels selectable", async () => {
    const user = userEvent.setup()
    api.listChatChannels.mockResolvedValue([
      channel(2, "Telegram", true),
      channel(3, "Lark", false),
    ])
    api.setFolderChatChannels.mockResolvedValue([3])
    renderDialog()

    const lark = await screen.findByRole("checkbox", { name: "Lark" })
    expect(lark).not.toBeDisabled()
    await user.click(lark)
    await user.click(screen.getByRole("button", { name: /save/i }))

    await waitFor(() =>
      expect(api.setFolderChatChannels).toHaveBeenCalledWith(1, [3])
    )
  })

  it("disables Save and does not show empty copy when load fails", async () => {
    api.listChatChannels.mockRejectedValue(new Error("network down"))
    renderDialog()

    expect(
      await screen.findByText(/Failed to load channels: network down/i)
    ).toBeInTheDocument()
    expect(screen.queryByText(/No chat channels yet/i)).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: /save/i })).toBeDisabled()
    expect(api.setFolderChatChannels).not.toHaveBeenCalled()
  })

  it("shows empty copy and allows Save after a successful empty load", async () => {
    api.listChatChannels.mockResolvedValue([])
    renderDialog()

    expect(await screen.findByText(/No chat channels yet/i)).toBeInTheDocument()
    expect(screen.getByRole("button", { name: /save/i })).toBeEnabled()
  })
})
