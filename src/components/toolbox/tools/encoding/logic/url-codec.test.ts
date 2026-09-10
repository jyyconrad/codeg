import { describe, expect, it } from "vitest"
import { EncodingError } from "./error"
import { decodeUrl, encodeUrl, parseUrl } from "./url-codec"

describe("url-codec", () => {
  const url = "https://example.com/search?q=hello world&lang=zh-CN"

  it("distinguishes encodeURI from encodeURIComponent", () => {
    expect(encodeUrl(url, "uri")).toBe(
      "https://example.com/search?q=hello%20world&lang=zh-CN"
    )
    expect(encodeUrl(url, "uri-component")).toBe(
      "https%3A%2F%2Fexample.com%2Fsearch%3Fq%3Dhello%20world%26lang%3Dzh-CN"
    )
  })

  it("round-trips both variants", () => {
    const samples = [url, "a b", "你", "/path?x=1"]
    for (const sample of samples) {
      expect(decodeUrl(encodeUrl(sample, "uri"), "uri")).toBe(sample)
      expect(
        decodeUrl(encodeUrl(sample, "uri-component"), "uri-component")
      ).toBe(sample)
    }
  })

  it("rejects malformed percent sequences", () => {
    expect(() => decodeUrl("%", "uri")).toThrow(EncodingError)
    expect(() => decodeUrl("%zz", "uri-component")).toThrow(EncodingError)
    expect(() => decodeUrl("%E0%", "uri")).toThrow(EncodingError)
  })

  it("parses protocol, host, path, and query", () => {
    expect(
      parseUrl("https://example.com:8080/a/b?x=1&x=2&y=hello+world")
    ).toEqual({
      protocol: "https",
      host: "example.com:8080",
      path: "/a/b",
      query: [
        { name: "x", value: "1" },
        { name: "x", value: "2" },
        { name: "y", value: "hello world" },
      ],
    })
  })

  it("rejects a string that is not a full URL", () => {
    expect(() => parseUrl("example.com/path")).toThrow(EncodingError)
    expect(() => parseUrl("")).toThrow(EncodingError)
  })
})
