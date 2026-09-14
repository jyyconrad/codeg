import { describe, expect, it } from "vitest"
import { TOOL_LOADERS } from "./tool-loaders"
import { TOOLBOX_TOOL_IDS } from "./types"

describe("TOOL_LOADERS", () => {
  it("covers every registered tool id", () => {
    expect(Object.keys(TOOL_LOADERS).sort()).toEqual(
      [...TOOLBOX_TOOL_IDS].sort()
    )
  })
})
