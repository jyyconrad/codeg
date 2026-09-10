import { describe, expect, it } from "vitest"
import { formatToolboxHashReport, type ToolboxHashReport } from "./toolbox-api"

describe("formatToolboxHashReport", () => {
  it("lists every algorithm and flags weak hashes", () => {
    const report = {
      MD5: "m",
      "SHA-1": "s1",
      "SHA-256": "s256",
      "SHA-384": "s384",
      "SHA-512": "s512",
      "SHA3-256": "s3",
      SM3: "sm",
    } satisfies ToolboxHashReport
    const text = formatToolboxHashReport(report)
    expect(text).toContain("MD5  (checksum only, not for passwords)")
    expect(text).toContain("SHA-256\ns256")
    expect(text).toContain("SM3\nsm")
  })
})
