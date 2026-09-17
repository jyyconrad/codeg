import { render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { NextIntlClientProvider } from "next-intl"
import { beforeEach, describe, expect, it, vi } from "vitest"

import enMessages from "@/i18n/messages/en.json"
import type { ChannelType, ChatChannelInfo } from "@/lib/types"

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

function channel(
  id: number,
  name: string,
  {
    enabled = true,
    type = "telegram",
    chatId,
  }: { enabled?: boolean; type?: ChannelType; chatId?: string } = {}
): ChatChannelInfo {
  return {
    id,
    name,
    channel_type: type,
    enabled,
    config_json:
      type === "weixin"
        ? JSON.stringify({ base_url: "https://example.test" })
        : JSON.stringify({ chat_id: chatId ?? `-100${id}` }),
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
      channel(3, "Lark", { type: "lark", chatId: "oc_default" }),
    ])
    api.listFolderChatChannels.mockResolvedValue([])
    api.setFolderChatChannels.mockResolvedValue([])
  })

  it("saves checked channels with empty session ids as the channel default", async () => {
    const user = userEvent.setup()
    renderDialog()

    await user.click(await screen.findByRole("checkbox", { name: "Telegram" }))
    await user.click(screen.getByRole("checkbox", { name: "Lark" }))
    await user.click(screen.getByRole("button", { name: /save/i }))

    await waitFor(() =>
      expect(api.setFolderChatChannels).toHaveBeenCalledWith(1, [
        { channel_id: 2, chat_id: null },
        { channel_id: 3, chat_id: null },
      ])
    )
  })

  it("saves a folder-specific session id for the selected channel", async () => {
    const user = userEvent.setup()
    renderDialog()

    await user.click(await screen.findByRole("checkbox", { name: "Telegram" }))
    await user.type(
      screen.getByRole("textbox", { name: /session id for telegram/i }),
      "-100999"
    )
    await user.click(screen.getByRole("button", { name: /save/i }))

    await waitFor(() =>
      expect(api.setFolderChatChannels).toHaveBeenCalledWith(1, [
        { channel_id: 2, chat_id: "-100999" },
      ])
    )
  })

  it("loads an existing session id into the input", async () => {
    api.listFolderChatChannels.mockResolvedValue([
      { channel_id: 2, chat_id: "-100888" },
    ])
    renderDialog()

    const checkbox = await screen.findByRole("checkbox", { name: "Telegram" })
    expect(checkbox).toBeChecked()
    expect(
      screen.getByRole("textbox", { name: /session id for telegram/i })
    ).toHaveValue("-100888")
  })

  it("hides the session id field until a channel is selected", async () => {
    renderDialog()

    await screen.findByRole("checkbox", { name: "Telegram" })
    expect(
      screen.queryByRole("textbox", { name: /session id/i })
    ).not.toBeInTheDocument()
  })

  it("does not show a session id field for weixin", async () => {
    api.listChatChannels.mockResolvedValue([
      channel(4, "Weixin", { type: "weixin" }),
    ])
    const user = userEvent.setup()
    renderDialog()

    await user.click(await screen.findByRole("checkbox", { name: "Weixin" }))
    expect(
      screen.queryByRole("textbox", { name: /session id/i })
    ).not.toBeInTheDocument()
  })

  it("omits disabled channels", async () => {
    api.listChatChannels.mockResolvedValue([
      channel(2, "Telegram", { enabled: true }),
      channel(3, "Lark", { enabled: false, type: "lark" }),
    ])
    renderDialog()

    expect(
      await screen.findByRole("checkbox", { name: "Telegram" })
    ).toBeInTheDocument()
    expect(
      screen.queryByRole("checkbox", { name: "Lark" })
    ).not.toBeInTheDocument()
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
