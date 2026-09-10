export const NAMING_STYLES = [
  "camelCase",
  "snake_case",
  "PascalCase",
  "kebab-case",
  "CONSTANT_CASE",
] as const

export type NamingStyle = (typeof NAMING_STYLES)[number]

function capitalize(token: string): string {
  if (!token) return token
  return token.charAt(0).toUpperCase() + token.slice(1)
}

export function tokenizeIdentifier(input: string): string[] {
  const spaced = input
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1 $2")
  return spaced
    .split(/[\s_\-]+/)
    .map((token) => token.trim())
    .filter((token) => token.length > 0)
    .map((token) => token.toLowerCase())
}

export function formatNamingTokens(
  tokens: string[],
  style: NamingStyle
): string {
  if (tokens.length === 0) return ""
  switch (style) {
    case "camelCase":
      return tokens[0] + tokens.slice(1).map(capitalize).join("")
    case "PascalCase":
      return tokens.map(capitalize).join("")
    case "snake_case":
      return tokens.join("_")
    case "kebab-case":
      return tokens.join("-")
    case "CONSTANT_CASE":
      return tokens.map((token) => token.toUpperCase()).join("_")
  }
}

export function convertNaming(text: string, style: NamingStyle): string {
  if (text === "") return ""
  return text
    .split(/\r\n|\n|\r/)
    .map((line) => formatNamingTokens(tokenizeIdentifier(line), style))
    .join("\n")
}
