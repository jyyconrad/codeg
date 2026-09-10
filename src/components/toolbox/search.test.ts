import { describe, expect, it } from "vitest"
import { listToolboxTools } from "./registry"
import { matchesToolboxQuery, searchToolboxTools } from "./search"
import { TOOLBOX_TOOL_IDS } from "./types"

function labels(title: string, extra = "") {
  return {
    title,
    description: extra,
    aliases: extra,
    category: "",
  }
}

describe("toolbox registry", () => {
  it("registers every v1 tool id exactly once", () => {
    const ids = listToolboxTools().map((tool) => tool.id)
    expect(ids).toEqual([...TOOLBOX_TOOL_IDS])
    expect(new Set(ids).size).toBe(TOOLBOX_TOOL_IDS.length)
  })
})

describe("toolbox search", () => {
  it("matches encoding aliases that users treat as encryption", () => {
    const base64 = listToolboxTools().find((tool) => tool.id === "base64")
    const hash = listToolboxTools().find((tool) => tool.id === "hash")
    expect(base64).toBeTruthy()
    expect(hash).toBeTruthy()
    expect(matchesToolboxQuery(base64!, labels("Base64"), "base64加密")).toBe(
      true
    )
    expect(matchesToolboxQuery(hash!, labels("Hash"), "md5加密")).toBe(true)
  })

  it("matches 国密 on the symmetric cipher tool", () => {
    const hits = searchToolboxTools("国密", (tool) =>
      labels(tool.id, tool.aliases.join(" "))
    )
    expect(hits.some((tool) => tool.id === "symmetric-cipher")).toBe(true)
  })

  it("returns every tool for an empty query", () => {
    expect(
      searchToolboxTools("", (tool) => labels(tool.id)).map((tool) => tool.id)
    ).toEqual([...TOOLBOX_TOOL_IDS])
  })
})
