import { EncodingError } from "./error"

export type UrlVariant = "uri" | "uri-component"

export interface UrlQueryParam {
  name: string
  value: string
}

export interface UrlParts {
  protocol: string
  host: string
  path: string
  query: UrlQueryParam[]
}

export function encodeUrl(input: string, variant: UrlVariant): string {
  if (input === "") return ""
  try {
    return variant === "uri" ? encodeURI(input) : encodeURIComponent(input)
  } catch {
    throw new EncodingError("Invalid URI sequence")
  }
}

export function decodeUrl(input: string, variant: UrlVariant): string {
  if (input === "") return ""
  try {
    return variant === "uri" ? decodeURI(input) : decodeURIComponent(input)
  } catch {
    throw new EncodingError("Invalid URI sequence")
  }
}

export function parseUrl(input: string): UrlParts {
  const trimmed = input.trim()
  if (trimmed === "") {
    throw new EncodingError("Invalid URL")
  }
  let url: URL
  try {
    url = new URL(trimmed)
  } catch {
    throw new EncodingError("Invalid URL")
  }
  const query: UrlQueryParam[] = []
  url.searchParams.forEach((value, name) => {
    query.push({ name, value })
  })
  return {
    protocol: url.protocol.replace(/:$/, ""),
    host: url.host,
    path: url.pathname,
    query,
  }
}

export function tryParseUrl(input: string): UrlParts | null {
  try {
    return parseUrl(input)
  } catch {
    return null
  }
}
