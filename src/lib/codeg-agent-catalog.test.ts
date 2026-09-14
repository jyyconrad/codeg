import { describe, expect, it } from "vitest"

import {
  catalogFromPlainSlug,
  catalogFromProviderModel,
  CODEG_CATALOG_DEFAULT_WINDOW,
  parseCodegAgentCatalog,
  serializeCodegAgentCatalog,
} from "./codeg-agent-catalog"

describe("codeg agent catalog", () => {
  it("parses kind=codeg_agent_catalog and keeps default when it exists", () => {
    const catalog = parseCodegAgentCatalog(
      JSON.stringify({
        kind: "codeg_agent_catalog",
        version: 1,
        default: "b",
        models: [
          { id: "a", name: "A", context_window: 32000 },
          { id: "b", name: "B", context_window: 64000 },
        ],
      })
    )
    expect(catalog?.default).toBe("b")
    expect(catalog?.protocol).toBe("chat_completions")
    expect(catalog?.models.map((row) => row.id)).toEqual(["a", "b"])
    expect(catalog?.models[1]?.context_window).toBe(64000)
  })

  it("falls back to the first model when default is missing", () => {
    const catalog = parseCodegAgentCatalog(
      JSON.stringify({
        kind: "codeg_agent_catalog",
        models: [{ id: "only" }],
      })
    )
    expect(catalog?.default).toBe("only")
    expect(catalog?.models[0]?.context_window).toBe(
      CODEG_CATALOG_DEFAULT_WINDOW
    )
  })

  it("ignores Claude and Codex JSON", () => {
    expect(
      parseCodegAgentCatalog(JSON.stringify({ main: "claude-sonnet-5" }))
    ).toBeNull()
    expect(
      parseCodegAgentCatalog(
        JSON.stringify({
          customs: [{ slug: "gpt-4.1", base: "gpt-4.1" }],
          default: "gpt-4.1",
        })
      )
    ).toBeNull()
  })

  it("upgrades a plain slug for the editor", () => {
    const catalog = catalogFromProviderModel("llama3.2")
    expect(catalog?.default).toBe("llama3.2")
    expect(catalog?.models).toHaveLength(1)
    expect(catalogFromPlainSlug("{")).toBeNull()
  })

  it("round-trips serialize → parse", () => {
    const catalog = catalogFromProviderModel("m")
    expect(catalog).not.toBeNull()
    const raw = serializeCodegAgentCatalog(catalog!)
    expect(parseCodegAgentCatalog(raw)).toEqual(catalog)
  })
})
