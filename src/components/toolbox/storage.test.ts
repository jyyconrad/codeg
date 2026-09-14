/** @vitest-environment jsdom */

import { beforeEach, describe, expect, it } from "vitest"
import {
  loadLastToolboxTool,
  loadToolboxFavorites,
  loadToolboxRecent,
  pushToolboxRecent,
  saveLastToolboxTool,
  saveToolboxFavorites,
} from "./storage"

describe("toolbox storage", () => {
  beforeEach(() => {
    localStorage.clear()
  })

  it("persists favorites without storing tool input", () => {
    saveToolboxFavorites(["json-format", "base64"])
    expect(loadToolboxFavorites()).toEqual(["json-format", "base64"])
    expect(JSON.stringify(localStorage)).not.toMatch(/secret|password|key/i)
  })

  it("drops unknown ids from persisted lists", () => {
    localStorage.setItem(
      "toolbox:favorites",
      JSON.stringify(["json-format", "not-a-tool", 3])
    )
    expect(loadToolboxFavorites()).toEqual(["json-format"])
  })

  it("keeps recent tools newest-first and capped", () => {
    for (let i = 0; i < 15; i += 1) {
      pushToolboxRecent("uuid")
    }
    pushToolboxRecent("json-format")
    const recent = loadToolboxRecent()
    expect(recent[0]).toBe("json-format")
    expect(recent.length).toBeLessThanOrEqual(12)
  })

  it("remembers the last opened tool id only", () => {
    saveLastToolboxTool("hmac")
    expect(loadLastToolboxTool()).toBe("hmac")
  })
})
