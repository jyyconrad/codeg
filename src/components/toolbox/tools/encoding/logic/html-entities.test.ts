import { describe, expect, it } from "vitest"
import { EncodingError } from "./error"
import { escapeHtml, unescapeHtml } from "./html-entities"

describe("html-entities", () => {
  it("escapes markup characters", () => {
    expect(escapeHtml(`<div class="x">A & B's</div>`)).toBe(
      "&lt;div class=&quot;x&quot;&gt;A &amp; B&#39;s&lt;/div&gt;"
    )
  })

  it("round-trips escaped text", () => {
    const samples = ["", "plain", `<div class="x">A & B's "C"</div>`, "你 & 我"]
    for (const sample of samples) {
      expect(unescapeHtml(escapeHtml(sample))).toBe(sample)
    }
  })

  it("unescapes named and numeric entities", () => {
    expect(unescapeHtml("&amp;&lt;&gt;&quot;&apos;")).toBe("&<>\"'")
    expect(unescapeHtml("&#60;&#x4f60;")).toBe("<你")
    expect(unescapeHtml("&nbsp;&copy;&euro;")).toBe("\u00a0©€")
  })

  it("rejects unknown or illegal entities", () => {
    expect(() => unescapeHtml("&notanentity;")).toThrow(EncodingError)
    expect(() => unescapeHtml("&#x110000;")).toThrow(EncodingError)
    expect(() => unescapeHtml("&#xD800;")).toThrow(EncodingError)
  })
})
