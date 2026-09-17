import {
  render,
  screen,
  fireEvent,
  waitFor,
  cleanup,
  within,
} from "@testing-library/react"
import userEvent from "@testing-library/user-event"
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
    turn_summary_builtin_prompt: "# wiki-turn-summary\nRead the current turn.",
    session_rollup_builtin_prompt: "# wiki-session-rollup\nReview the session.",
    synthesize_builtin_prompt: "# wiki-synthesize\nUpdate existing topics.",
    turn_summary_builtin_task: "Read {source_path} and summarize this turn.",
    session_rollup_builtin_task:
      "Read {memory_paths} and summarize the session.",
    synthesize_builtin_task: "Read {memory_paths} and update {wiki_root}.",
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
  backend.save.mockImplementation(async (settings: WikiSettingsPayload) => ({
    ...initial(),
    ...settings,
  }))
  backend.providers.mockResolvedValue([
    { id: 1, name: "Wiki provider", agent_type: "codeg_agent", model: "one" },
    { id: 2, name: "Other provider", agent_type: "codeg_agent", model: "two" },
  ])
})

describe("Wiki stage skills and task instructions", () => {
  it.each([
    ["turn_summary", "turnSummaryPrompt"],
    ["session_rollup", "sessionRollupPrompt"],
    ["synthesize", "synthesizePrompt"],
  ] as const)(
    "edits and restores the %s skill without saving task templates",
    async (slot, label) => {
      const user = userEvent.setup()
      renderSettings()
      await screen.findByLabelText(messages.Wiki.v2.dailyTime)
      await user.click(screen.getByText(messages.Wiki.v2.advancedSettings))
      const stage = within(
        screen.getByRole("region", { name: messages.Wiki.v2.stages[slot] })
      )
      const skill = stage.getByLabelText(messages.WikiSettings[label])
      expect(skill).toHaveValue(initial()[`${slot}_builtin_prompt`])
      expect(skill).not.toHaveAttribute("readonly")

      const taskToggle = stage.getByText(messages.Wiki.v2.builtinTask)
      expect(taskToggle.closest("details")).not.toHaveAttribute("open")
      await user.click(taskToggle)
      const preview = stage.getByRole("textbox", {
        name: messages.Wiki.v2.builtinTask,
      })
      expect(preview).toHaveAttribute("readonly")
      await user.type(preview, "Attempted edit")
      expect(preview).toHaveValue(initial()[`${slot}_builtin_task`])
      expect(
        screen.getByRole("button", { name: messages.WikiSettings.save })
      ).toBeDisabled()

      fireEvent.change(skill, {
        target: { value: "# Custom skill\nKeep decisions." },
      })
      await user.click(
        screen.getByRole("button", { name: messages.WikiSettings.save })
      )
      await screen.findByRole("status")
      const payload = backend.save.mock.calls[0][0]
      expect(payload[slot].prompt).toBe("# Custom skill\nKeep decisions.")
      for (const key of ["turn_summary", "session_rollup", "synthesize"]) {
        expect(payload).not.toHaveProperty(`${key}_builtin_task`)
        expect(payload).not.toHaveProperty(`${key}_builtin_prompt`)
      }
      expect(preview).toHaveValue(initial()[`${slot}_builtin_task`])

      await user.click(
        stage.getByRole("button", {
          name: messages.WikiSettings.restoreBuiltinPrompt,
        })
      )
      expect(skill).toHaveValue(initial()[`${slot}_builtin_prompt`])
      await user.click(
        screen.getByRole("button", { name: messages.WikiSettings.save })
      )
      await waitFor(() => expect(backend.save).toHaveBeenCalledTimes(2))
      expect(backend.save.mock.calls[1][0][slot].prompt).toBeNull()
    }
  )

  it("omits task previews when the backend does not provide templates", async () => {
    const value = initial()
    delete value.turn_summary_builtin_task
    delete value.session_rollup_builtin_task
    delete value.synthesize_builtin_task
    backend.settings.mockResolvedValue(value)
    renderSettings()
    await screen.findByLabelText(messages.Wiki.v2.dailyTime)
    fireEvent.click(screen.getByText(messages.Wiki.v2.advancedSettings))
    expect(
      screen.queryByText(messages.Wiki.v2.builtinTask)
    ).not.toBeInTheDocument()
    expect(
      screen.getByLabelText(messages.WikiSettings.turnSummaryPrompt)
    ).toHaveValue(value.turn_summary_builtin_prompt)
  })
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
    expect(
      document.getElementById("wiki-main-model-provider")
    ).toHaveTextContent(messages.Wiki.v2.savedProviderUnavailable)
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
