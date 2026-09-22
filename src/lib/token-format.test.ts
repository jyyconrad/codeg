import { describe, expect, it } from "vitest"

import { formatTokenCount } from "./token-format"

describe("formatTokenCount", () => {
  it("keeps counts below 1000 as a plain integer", () => {
    expect(formatTokenCount(0)).toBe("0")
    expect(formatTokenCount(999)).toBe("999")
  })

  it("uses K / M / B compact units", () => {
    expect(formatTokenCount(1_000)).toBe("1K")
    expect(formatTokenCount(1_250)).toBe("1.3K")
    expect(formatTokenCount(1_000_000)).toBe("1M")
    expect(formatTokenCount(2_400_000)).toBe("2.4M")
    expect(formatTokenCount(1_000_000_000)).toBe("1B")
    expect(formatTokenCount(1_250_000_000)).toBe("1.3B")
  })
})
