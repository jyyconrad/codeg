import { describe, expect, it, vi } from "vitest"

import {
  CODEG_BUILTIN_COMPACT_PROMPT,
  CODEG_BUILTIN_SYSTEM_PROMPT,
} from "./codeg-agent-prompts"
import {
  bindCodegProviderEnv,
  bindCodegProviderWithProbe,
  CODEG_COMPACT_RECENT_TURNS_KEY,
  CODEG_COMPACT_SOFT_PERCENT_KEY,
  CODEG_DEFAULT_COMPACT_RECENT_TURNS,
  CODEG_DEFAULT_COMPACT_SOFT_PERCENT,
  CODEG_DEFAULT_MAX_TURNS,
  CODEG_INJECT_AGENTS_MD_KEY,
  CODEG_INJECT_CLAUDE_MD_KEY,
  CODEG_INJECT_TREE_KEY,
  CODEG_MAX_TURNS_KEY,
  CODEG_SYSTEM_PROMPT_KEY,
  codegDraftFromEnv,
  codegEnvInt,
  codegFlag,
  ensureCodegLaunchEnv,
  overlayCodegPromptEnv,
  parseCodegContextWindows,
  patchCodegContextWindow,
  patchCodegEnvInt,
  patchCodegFlag,
  persistThenRunPreflight,
} from "./codeg-agent-env"

describe("codeg agent env helpers", () => {
  it("ships a resumable compact prompt that writes session markdown and hands off in prose", () => {
    expect(CODEG_BUILTIN_COMPACT_PROMPT).toContain("current work goal")
    expect(CODEG_BUILTIN_COMPACT_PROMPT).toContain("unfinished")
    expect(CODEG_BUILTIN_COMPACT_PROMPT).toContain("Markdown")
    expect(CODEG_BUILTIN_COMPACT_PROMPT).toContain("write_file")
    expect(CODEG_BUILTIN_COMPACT_PROMPT).toContain("tool loop")
    expect(CODEG_BUILTIN_COMPACT_PROMPT).not.toContain('"summary"')
    expect(CODEG_BUILTIN_COMPACT_PROMPT).not.toContain("Return JSON")
  })

  it("ships an OpenCode-style main prompt with Codeg tools and file links", () => {
    expect(CODEG_BUILTIN_SYSTEM_PROMPT).toContain("You are Codeg Agent")
    expect(CODEG_BUILTIN_SYSTEM_PROMPT).toContain("read_file")
    expect(CODEG_BUILTIN_SYSTEM_PROMPT).toContain("subagent")
    expect(CODEG_BUILTIN_SYSTEM_PROMPT).toContain(
      "[使用手册.docx](docs/使用手册.docx)"
    )
    expect(CODEG_BUILTIN_SYSTEM_PROMPT).toContain("Output:")
  })

  it("treats inject flags as off by default and writes 1 when on", () => {
    expect(codegFlag("", CODEG_INJECT_AGENTS_MD_KEY)).toBe(false)
    const on = patchCodegFlag("", CODEG_INJECT_AGENTS_MD_KEY, true)
    expect(on).toContain(`${CODEG_INJECT_AGENTS_MD_KEY}=1`)
    expect(codegFlag(on, CODEG_INJECT_AGENTS_MD_KEY)).toBe(true)
    expect(
      codegFlag("CODEG_AGENT_INJECT_TREE=yes", CODEG_INJECT_TREE_KEY)
    ).toBe(true)
    const off = patchCodegFlag(on, CODEG_INJECT_AGENTS_MD_KEY, false)
    expect(off).not.toContain(CODEG_INJECT_AGENTS_MD_KEY)
    expect(
      codegFlag("CODEG_AGENT_INJECT_CLAUDE_MD=no", CODEG_INJECT_CLAUDE_MD_KEY)
    ).toBe(false)
  })

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

  it("treats built-in prompt text as delete so spawn keeps the Rust default", () => {
    const overlaid = overlayCodegPromptEnv(
      {
        [CODEG_SYSTEM_PROMPT_KEY]: "custom",
        CODEG_AGENT_COMPACT_PROMPT: "custom compact",
      },
      CODEG_BUILTIN_SYSTEM_PROMPT,
      CODEG_BUILTIN_COMPACT_PROMPT
    )
    expect(overlaid[CODEG_SYSTEM_PROMPT_KEY]).toBeUndefined()
    expect(overlaid.CODEG_AGENT_COMPACT_PROMPT).toBeUndefined()
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

  it("prefills the built-in prompt bodies when env keys are absent", () => {
    const draft = codegDraftFromEnv({ CODEG_AGENT_MODEL: "llama3" })
    expect(draft.systemPrompt).toBe(CODEG_BUILTIN_SYSTEM_PROMPT)
    expect(draft.compactPrompt).toBe(CODEG_BUILTIN_COMPACT_PROMPT)
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

  it("replaces context windows with every catalog model on bind", () => {
    const bound = bindCodegProviderEnv(
      'CODEG_AGENT_CONTEXT_WINDOWS={"old":8000}',
      {
        api_url: "https://api.example.com/v1",
        api_key: "sk",
        agent_type: "codeg_agent",
        model: JSON.stringify({
          kind: "codeg_agent_catalog",
          version: 1,
          default: "b",
          models: [
            { id: "a", name: "a", context_window: 32000 },
            { id: "b", name: "b", context_window: 64000 },
          ],
        }),
      }
    )
    expect(bound.model).toBe("b")
    expect(parseCodegContextWindows(bound.envText)).toEqual({
      a: 32000,
      b: 64000,
    })
    expect(bound.envText).not.toContain('"old"')
  })

  it("writes the catalog protocol on bind and clears it on unbind", () => {
    const catalog = {
      kind: "codeg_agent_catalog" as const,
      version: 1 as const,
      protocol: "responses" as const,
      default: "b",
      models: [
        { id: "a", name: "a", context_window: 32000 },
        { id: "b", name: "b", context_window: 64000 },
      ],
    }
    const bound = bindCodegProviderEnv("", {
      api_url: "https://api.example.com/v1",
      api_key: "sk",
      agent_type: "codeg_agent",
      model: JSON.stringify(catalog),
    })
    expect(bound.envText).toContain("CODEG_AGENT_PROTOCOL=responses")
    const unbound = bindCodegProviderEnv(bound.envText, null)
    expect(unbound.envText).not.toContain("CODEG_AGENT_PROTOCOL=")
  })

  it("probes on bind and locks the detected protocol onto the catalog", async () => {
    const probe = vi.fn().mockResolvedValue("responses")
    const result = await bindCodegProviderWithProbe(
      "",
      {
        api_url: "https://gw.example/v1",
        api_key: "sk",
        agent_type: "codeg_agent",
        model: JSON.stringify({
          kind: "codeg_agent_catalog",
          version: 1,
          protocol: "auto",
          default: "gateway-model",
          models: [
            {
              id: "gateway-model",
              name: "gateway-model",
              context_window: 128000,
            },
          ],
        }),
      },
      probe
    )
    expect(probe).toHaveBeenCalledWith({
      baseUrl: "https://gw.example/v1",
      apiKey: "sk",
      modelId: "gateway-model",
    })
    expect(result.protocol).toBe("responses")
    expect(result.catalog?.protocol).toBe("responses")
    expect(result.envText).toContain("CODEG_AGENT_PROTOCOL=responses")
  })

  it("does not probe when unbinding", async () => {
    const probe = vi.fn()
    const result = await bindCodegProviderWithProbe(
      "CODEG_AGENT_PROTOCOL=responses",
      null,
      probe
    )
    expect(probe).not.toHaveBeenCalled()
    expect(result.protocol).toBe("")
    expect(result.catalog).toBeNull()
    expect(result.envText).not.toContain("CODEG_AGENT_PROTOCOL=")
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
