import { describe, expect, it } from "vitest"
import {
  dailyCronTime,
  cronForDailyTime,
  effectiveWikiPrompt,
  normalizeWikiSettings,
  wikiProjectDisplayTitle,
  wikiSettingsPayload,
  wikiSourceTitle,
  wikiSynthesizeEnabled,
} from "./wiki-types"

describe("wiki settings prompts", () => {
  it("round-trips an independent provider binding for every model slot", () => {
    const view = normalizeWikiSettings({
      turn_summary: { provider_id: 7, model_id: "m1", prompt: null },
      session_rollup: { provider_id: 8, model_id: "m2", prompt: null },
      synthesize: {
        enabled: true,
        provider_id: 9,
        model_id: "m3",
        prompt: null,
      },
    })
    expect(wikiSettingsPayload(view).turn_summary).toMatchObject({
      provider_id: 7,
      model_id: "m1",
    })
    expect(wikiSettingsPayload(view).session_rollup).toMatchObject({
      provider_id: 8,
      model_id: "m2",
    })
    expect(wikiSettingsPayload(view).synthesize).toMatchObject({
      provider_id: 9,
      model_id: "m3",
    })
  })

  it("saves null when the edited prompt matches the builtin skill", () => {
    const builtin = "# wiki-turn-summary\nnever rewrite"
    const view = normalizeWikiSettings({
      turn_summary: { model_id: null, prompt: builtin },
      session_rollup: { model_id: null, prompt: "  " },
      synthesize: { enabled: true, model_id: null, prompt: "  " },
      turn_summary_builtin_prompt: builtin,
      session_rollup_builtin_prompt: "# wiki-session-rollup",
      synthesize_builtin_prompt: "# wiki-synthesize",
    })
    const payload = wikiSettingsPayload(view)
    expect(payload.turn_summary.prompt).toBeNull()
    expect(payload.session_rollup.prompt).toBeNull()
    expect(payload.synthesize.prompt).toBeNull()
    expect(payload).not.toHaveProperty("ingest")
    expect(payload).not.toHaveProperty("compile")
  })

  it("keeps a custom full prompt", () => {
    const view = normalizeWikiSettings({
      turn_summary: { model_id: null, prompt: "# custom turn" },
      turn_summary_builtin_prompt: "# builtin turn",
    })
    expect(wikiSettingsPayload(view).turn_summary.prompt).toBe("# custom turn")
    expect(effectiveWikiPrompt(null, "# builtin turn")).toBe("# builtin turn")
    expect(effectiveWikiPrompt("# custom", "# builtin turn")).toBe("# custom")
  })

  it("does not retain retired settings or let them disable the new schedule", () => {
    const legacy = JSON.parse(
      '{"ingest":{"model_id":"old"},"compile":{"enabled":false}}'
    )
    const view = normalizeWikiSettings(legacy)
    expect(view.synthesize.enabled).toBe(true)
    expect(view.synthesize.model_id).toBeNull()
    expect(wikiSettingsPayload(view)).not.toHaveProperty("compile")
    expect(wikiSynthesizeEnabled(view)).toBe(true)
  })
})

describe("wiki display titles", () => {
  it("prefers the folder name over Project {id}", () => {
    expect(
      wikiProjectDisplayTitle({
        id: "p1",
        vault_id: "v",
        db_instance_id: "d",
        root_folder_id: 2,
        root_folder_name: "switchgear",
      })
    ).toBe("switchgear")
    expect(
      wikiProjectDisplayTitle({
        id: "p1",
        vault_id: "v",
        db_instance_id: "d",
        root_folder_id: 2,
      })
    ).toBe("Project 2")
  })

  it("uses the source title and leaves missing titles for the UI fallback", () => {
    expect(
      wikiSourceTitle({
        id: "bb4c8465-288e-4d79-8385-d3e7d5a94e3b",
        source_title: "方案 C 样本流诊断窗分批改造",
      })
    ).toBe("方案 C 样本流诊断窗分批改造")
    expect(
      wikiSourceTitle({
        id: "bb4c8465-288e-4d79-8385-d3e7d5a94e3b",
      })
    ).toBeNull()
  })
})

describe("Wiki schedules", () => {
  it("preserves custom schedules and generates daily schedules only from valid times", () => {
    expect(dailyCronTime("15 3 * * *")).toBe("03:15")
    expect(dailyCronTime("0 */4 * * *")).toBeNull()
    expect(cronForDailyTime("23:59")).toBe("59 23 * * *")
    expect(cronForDailyTime("25:00")).toBeNull()
  })
})
