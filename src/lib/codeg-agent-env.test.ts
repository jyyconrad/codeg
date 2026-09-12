import { describe, expect, it } from "vitest"

import {
  bindCodegProviderEnv,
  CODEG_COMPACT_RECENT_TURNS_KEY,
  CODEG_COMPACT_SOFT_PERCENT_KEY,
  CODEG_DEFAULT_COMPACT_RECENT_TURNS,
  CODEG_DEFAULT_COMPACT_SOFT_PERCENT,
  CODEG_DEFAULT_MAX_TURNS,
  CODEG_MAX_TURNS_KEY,
  CODEG_SYSTEM_PROMPT_KEY,
  codegDraftFromEnv,
  codegEnvInt,
  ensureCodegLaunchEnv,
  overlayCodegPromptEnv,
  parseCodegContextWindows,
  patchCodegContextWindow,
  patchCodegEnvInt,
  persistThenRunPreflight,
} from "./codeg-agent-env"

describe("codeg agent env helpers", () => {
  it("writes a window on bind and keeps an existing one", () => {
    const env = ensureCodegLaunchEnv("", "gateway-model", 128000)
    expect(parseCodegContextWindows(env)).toEqual({ "gateway-model": 128000 })
    expect(env).toContain("CODEG_AGENT_MAX_OUTPUT_TOKENS=4096")
    const kept = ensureCodegLaunchEnv(
      'CODEG_AGENT_CONTEXT_WINDOWS={"gateway-model":64000}',
      "gateway-model",
      128000
    )
    expect(kept).toContain("64000")
    expect(kept).not.toContain("128000")
  })

  it("overlays empty prompts as delete", () => {
    const overlaid = overlayCodegPromptEnv(
      { KEEP: "1", [CODEG_SYSTEM_PROMPT_KEY]: "old" },
      "  ",
      "Keep paths"
    )
    expect(overlaid[CODEG_SYSTEM_PROMPT_KEY]).toBeUndefined()
    expect(overlaid.CODEG_AGENT_COMPACT_PROMPT).toBe("Keep paths")
    expect(overlaid.KEEP).toBe("1")
  })

  it("strips multiline prompts from the KEY=VALUE draft", () => {
    const draft = codegDraftFromEnv({
      CODEG_AGENT_MODEL: "llama3",
      [CODEG_SYSTEM_PROMPT_KEY]: "Be terse.",
      CODEG_AGENT_COMPACT_PROMPT: "Keep paths",
    })
    expect(draft.systemPrompt).toBe("Be terse.")
    expect(draft.compactPrompt).toBe("Keep paths")
    expect(draft.envText).toContain("CODEG_AGENT_MODEL=llama3")
    expect(draft.envText).not.toContain("Be terse.")
  })

  it("binds a Completions provider and projects URL / key / model", () => {
    const bound = bindCodegProviderEnv("", {
      api_url: "http://127.0.0.1:11434/v1",
      api_key: "",
      agent_type: "codeg_agent",
      model: "llama3.2",
    })
    expect(bound.model).toBe("llama3.2")
    expect(bound.envText).toContain(
      "CODEG_AGENT_API_BASE_URL=http://127.0.0.1:11434/v1"
    )
    expect(bound.envText).not.toContain("CODEG_AGENT_API_KEY=")
    expect(bound.envText).toContain("CODEG_AGENT_MODEL=llama3.2")
    expect(parseCodegContextWindows(bound.envText)["llama3.2"]).toBe(128000)
  })

  it("reads and patches compact / runtime integers with defaults", () => {
    expect(
      codegEnvInt(
        "",
        CODEG_COMPACT_SOFT_PERCENT_KEY,
        CODEG_DEFAULT_COMPACT_SOFT_PERCENT
      )
    ).toBe("80")
    const patched = patchCodegEnvInt(
      "",
      CODEG_MAX_TURNS_KEY,
      "24",
      CODEG_DEFAULT_MAX_TURNS
    )
    expect(
      codegEnvInt(patched, CODEG_MAX_TURNS_KEY, CODEG_DEFAULT_MAX_TURNS)
    ).toBe("24")
    const cleared = patchCodegEnvInt(
      patched,
      CODEG_COMPACT_RECENT_TURNS_KEY,
      "  ",
      CODEG_DEFAULT_COMPACT_RECENT_TURNS
    )
    expect(
      codegEnvInt(
        cleared,
        CODEG_COMPACT_RECENT_TURNS_KEY,
        CODEG_DEFAULT_COMPACT_RECENT_TURNS
      )
    ).toBe("6")
  })

  it("patches a single model window without inventing siblings", () => {
    const env = patchCodegContextWindow("", "gateway-model", 64000)
    expect(parseCodegContextWindows(env)).toEqual({ "gateway-model": 64000 })
  })

  it("runs persist then preflight in order", async () => {
    const order: string[] = []
    await persistThenRunPreflight(
      async () => {
        order.push("persist")
      },
      async () => {
        order.push("preflight")
      }
    )
    expect(order).toEqual(["persist", "preflight"])
  })
})
