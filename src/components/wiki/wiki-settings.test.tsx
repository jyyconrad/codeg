import {
  render,
  screen,
  fireEvent,
  waitFor,
  cleanup,
} from "@testing-library/react"
import { NextIntlClientProvider } from "next-intl"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import messages from "@/i18n/messages/en.json"
import { WikiSettings } from "@/components/settings/wiki-settings"
import {
  normalizeWikiSettings,
  type WikiSettings as WikiSettingsPayload,
} from "@/lib/wiki-types"
const backend = vi.hoisted(() => ({
  settings: vi.fn(),
  save: vi.fn(),
  providers: vi.fn(),
}))
vi.mock("@/lib/wiki-api", () => ({
  getWikiSettings: backend.settings,
  updateWikiSettings: backend.save,
}))
vi.mock("@/lib/api", () => ({
  listModelProviders: backend.providers,
  listAllFolderDetails: async () => [],
}))
const initial = () =>
  normalizeWikiSettings({
    enabled: true,
    turn_summary: { provider_id: 1, model_id: "one" },
    session_rollup: { provider_id: 2, model_id: "two" },
    synthesize: { provider_id: 1, model_id: "one", enabled: true },
  })
function renderSettings() {
  return render(
    <NextIntlClientProvider locale="en" messages={messages}>
      <WikiSettings />
    </NextIntlClientProvider>
  )
}
beforeEach(() => {
  backend.settings.mockResolvedValue(initial())
  backend.save.mockImplementation(
    async (settings: WikiSettingsPayload) => settings
  )
  backend.providers.mockResolvedValue([
    { id: 1, name: "Wiki provider", agent_type: "codeg_agent", model: "one" },
    { id: 2, name: "Other provider", agent_type: "codeg_agent", model: "two" },
  ])
})
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})
describe("independent Wiki model settings", () => {
  it("preserves each saved stage binding when saving a schedule change", async () => {
    renderSettings()
    const time = await screen.findByLabelText(messages.Wiki.v2.dailyTime)
    fireEvent.change(time, { target: { value: "04:12" } })
    fireEvent.click(
      screen.getByRole("button", { name: messages.WikiSettings.save })
    )
    await screen.findByRole("status")
    expect(backend.save).toHaveBeenCalledWith(
      expect.objectContaining({
        compile_cron: "12 4 * * *",
        turn_summary: expect.objectContaining({
          provider_id: 1,
          model_id: "one",
        }),
        session_rollup: expect.objectContaining({
          provider_id: 2,
          model_id: "two",
        }),
        synthesize: expect.objectContaining({
          provider_id: 1,
          model_id: "one",
        }),
      })
    )
  })
  it("does not silently replace an unavailable saved provider", async () => {
    const value = initial()
    value.synthesize.provider_id = 99
    backend.settings.mockResolvedValue(value)
    renderSettings()
    await screen.findByLabelText(messages.Wiki.v2.dailyTime)
    expect(document.getElementById("wiki-main-model-provider")).toHaveTextContent(
      messages.Wiki.v2.savedProviderUnavailable
    )
    fireEvent.change(screen.getByLabelText(messages.Wiki.v2.dailyTime), {
      target: { value: "04:12" },
    })
    expect(
      screen.getByRole("button", { name: messages.WikiSettings.save })
    ).toBeDisabled()
  })
  it("retains unsaved choices after a failed save and allows retry", async () => {
    backend.save.mockRejectedValueOnce(new Error("Unavailable"))
    renderSettings()
    const time = await screen.findByLabelText(messages.Wiki.v2.dailyTime)
    fireEvent.change(time, { target: { value: "04:12" } })
    fireEvent.click(
      screen.getByRole("button", { name: messages.WikiSettings.save })
    )
    await screen.findByRole("alert")
    expect(time).toHaveValue("04:12")
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: messages.WikiSettings.save })
      ).toBeEnabled()
    )
  })
})
