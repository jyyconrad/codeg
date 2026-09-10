export const TOOLBOX_CATEGORIES = [
  "text",
  "dev",
  "encoding",
  "crypto",
  "image",
  "general",
] as const

export type ToolboxCategory = (typeof TOOLBOX_CATEGORIES)[number]

export const TOOLBOX_TOOL_IDS = [
  "text-stats",
  "text-clean",
  "line-dedupe-sort",
  "case-naming",
  "text-diff",
  "find-replace",
  "markdown-preview",
  "zh-convert",
  "json-format",
  "timestamp",
  "uuid",
  "json-yaml",
  "json-csv",
  "regex-tester",
  "cron",
  "sql-format",
  "code-format",
  "base64",
  "url-codec",
  "hex-string",
  "html-entities",
  "unicode-escape",
  "radix",
  "symmetric-cipher",
  "hash",
  "hmac",
  "password-gen",
  "asymmetric-cipher",
  "jwt",
  "password-strength",
  "totp",
  "bcrypt",
  "cert-pem",
  "image-compress",
  "color-convert",
  "image-crop",
  "image-watermark",
  "image-stitch",
  "qr-generate",
  "qr-decode",
  "unit-convert",
  "date-diff",
] as const

export type ToolboxToolId = (typeof TOOLBOX_TOOL_IDS)[number]

export function isToolboxToolId(value: string): value is ToolboxToolId {
  return (TOOLBOX_TOOL_IDS as readonly string[]).includes(value)
}

export interface ToolboxToolMeta {
  id: ToolboxToolId
  category: ToolboxCategory
  /** Extra search needles (locale-independent), e.g. "md5加密". */
  aliases: readonly string[]
  /** Tools that commonly accept this tool's text result. */
  chainTargets: readonly ToolboxToolId[]
}
