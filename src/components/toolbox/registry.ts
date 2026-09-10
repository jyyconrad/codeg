import type { ToolboxCategory, ToolboxToolId, ToolboxToolMeta } from "./types"

const META: readonly ToolboxToolMeta[] = [
  {
    id: "text-stats",
    category: "text",
    aliases: ["word count", "字数统计"],
    chainTargets: [],
  },
  {
    id: "text-clean",
    category: "text",
    aliases: ["trim", "文本清洗"],
    chainTargets: ["line-dedupe-sort", "text-stats"],
  },
  {
    id: "line-dedupe-sort",
    category: "text",
    aliases: ["unique", "去重", "排序"],
    chainTargets: ["text-stats"],
  },
  {
    id: "case-naming",
    category: "text",
    aliases: ["camelCase", "snake_case", "命名"],
    chainTargets: [],
  },
  {
    id: "text-diff",
    category: "text",
    aliases: ["diff", "对比"],
    chainTargets: [],
  },
  {
    id: "find-replace",
    category: "text",
    aliases: ["replace", "查找替换"],
    chainTargets: ["text-clean"],
  },
  {
    id: "markdown-preview",
    category: "text",
    aliases: ["md", "markdown"],
    chainTargets: [],
  },
  {
    id: "zh-convert",
    category: "text",
    aliases: ["简繁", "opencc"],
    chainTargets: [],
  },
  {
    id: "json-format",
    category: "dev",
    aliases: ["json", "pretty"],
    chainTargets: ["json-yaml", "json-csv"],
  },
  {
    id: "timestamp",
    category: "dev",
    aliases: ["unix", "时间戳", "epoch"],
    chainTargets: [],
  },
  {
    id: "uuid",
    category: "dev",
    aliases: ["guid"],
    chainTargets: [],
  },
  {
    id: "json-yaml",
    category: "dev",
    aliases: ["yaml", "yml"],
    chainTargets: ["json-format"],
  },
  {
    id: "json-csv",
    category: "dev",
    aliases: ["csv"],
    chainTargets: ["json-format"],
  },
  {
    id: "regex-tester",
    category: "dev",
    aliases: ["regexp", "正则"],
    chainTargets: [],
  },
  {
    id: "cron",
    category: "dev",
    aliases: ["crontab", "定时"],
    chainTargets: [],
  },
  {
    id: "sql-format",
    category: "dev",
    aliases: ["sql"],
    chainTargets: [],
  },
  {
    id: "code-format",
    category: "dev",
    aliases: ["xml", "yaml viewer"],
    chainTargets: [],
  },
  {
    id: "base64",
    category: "encoding",
    aliases: ["base64加密", "base64解密"],
    chainTargets: ["symmetric-cipher", "json-format", "hex-string"],
  },
  {
    id: "url-codec",
    category: "encoding",
    aliases: ["urlencode", "uri"],
    chainTargets: ["json-format"],
  },
  {
    id: "hex-string",
    category: "encoding",
    aliases: ["hex", "十六进制"],
    chainTargets: ["base64", "symmetric-cipher"],
  },
  {
    id: "html-entities",
    category: "encoding",
    aliases: ["html escape"],
    chainTargets: [],
  },
  {
    id: "unicode-escape",
    category: "encoding",
    aliases: ["\\u", "unicode"],
    chainTargets: ["json-format"],
  },
  {
    id: "radix",
    category: "encoding",
    aliases: ["binary", "进制"],
    chainTargets: [],
  },
  {
    id: "symmetric-cipher",
    category: "crypto",
    aliases: ["aes", "sm4", "aes加密", "国密"],
    chainTargets: ["json-format", "base64", "hex-string"],
  },
  {
    id: "hash",
    category: "crypto",
    aliases: ["md5", "sha256", "sm3", "md5加密"],
    chainTargets: [],
  },
  {
    id: "hmac",
    category: "crypto",
    aliases: ["sign", "签名"],
    chainTargets: [],
  },
  {
    id: "password-gen",
    category: "crypto",
    aliases: ["password", "密码生成"],
    chainTargets: ["password-strength"],
  },
  {
    id: "asymmetric-cipher",
    category: "crypto",
    aliases: ["rsa", "sm2", "公钥"],
    chainTargets: ["base64"],
  },
  {
    id: "jwt",
    category: "crypto",
    aliases: ["json web token"],
    chainTargets: ["json-format"],
  },
  {
    id: "password-strength",
    category: "crypto",
    aliases: ["entropy", "密码强度"],
    chainTargets: [],
  },
  {
    id: "totp",
    category: "crypto",
    aliases: ["otp", "2fa"],
    chainTargets: [],
  },
  {
    id: "image-compress",
    category: "image",
    aliases: ["compress", "图片压缩"],
    chainTargets: [],
  },
  {
    id: "color-convert",
    category: "image",
    aliases: ["hex", "rgb", "hsl"],
    chainTargets: [],
  },
  {
    id: "image-crop",
    category: "image",
    aliases: ["crop", "resize", "裁剪"],
    chainTargets: [],
  },
  {
    id: "image-watermark",
    category: "image",
    aliases: ["watermark", "水印"],
    chainTargets: [],
  },
  {
    id: "image-stitch",
    category: "image",
    aliases: ["concat", "拼接"],
    chainTargets: [],
  },
  {
    id: "qr-generate",
    category: "general",
    aliases: ["qrcode", "二维码"],
    chainTargets: [],
  },
  {
    id: "qr-decode",
    category: "general",
    aliases: ["scan", "扫码"],
    chainTargets: ["url-codec", "json-format"],
  },
  {
    id: "unit-convert",
    category: "general",
    aliases: ["unit", "单位"],
    chainTargets: [],
  },
  {
    id: "date-diff",
    category: "general",
    aliases: ["datediff", "日期间隔"],
    chainTargets: [],
  },
]

const BY_ID = new Map(META.map((tool) => [tool.id, tool]))

export function listToolboxTools(): readonly ToolboxToolMeta[] {
  return META
}

export function getToolboxTool(id: string): ToolboxToolMeta | undefined {
  return BY_ID.get(id as ToolboxToolId)
}

export function listToolboxToolsByCategory(
  category: ToolboxCategory
): readonly ToolboxToolMeta[] {
  return META.filter((tool) => tool.category === category)
}

export { isToolboxToolId } from "./types"
