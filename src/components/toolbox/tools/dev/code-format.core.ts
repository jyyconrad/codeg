import { dump, JSON_SCHEMA, load, YAMLException } from "js-yaml"

export type CodeFormatKind = "xml" | "yaml"

export type CodeFormatResult =
  | { ok: true; output: string }
  | { ok: false; error: string }

function escapeXmlText(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
}

function escapeXmlAttr(value: string): string {
  return escapeXmlText(value).replace(/"/g, "&quot;")
}

function yamlErrorMessage(error: unknown): string {
  if (error instanceof YAMLException) {
    const line = error.mark ? error.mark.line + 1 : undefined
    const column = error.mark ? error.mark.column + 1 : undefined
    const reason = error.reason || error.message
    if (line != null && column != null) {
      return `at line ${line}, column ${column}: ${reason}`
    }
    return reason
  }
  return error instanceof Error ? error.message : String(error)
}

export function indentXmlFallback(xml: string): string {
  const normalized = xml.replace(/>\s*</g, ">\n<").trim()
  if (normalized === "") return ""
  const lines: string[] = []
  let depth = 0
  for (const raw of normalized.split("\n")) {
    const line = raw.trim()
    const isClosing = /^<\//.test(line)
    const isDecl = /^<\?/.test(line) || /^<!/.test(line)
    const isSelfClosing = /\/>$/.test(line)
    const isOpenAndClose = /^<[^>]+>.*<\/[^>]+>$/.test(line)
    if (isClosing) depth = Math.max(0, depth - 1)
    lines.push(`${"  ".repeat(depth)}${line}`)
    if (
      !isClosing &&
      !isDecl &&
      !isSelfClosing &&
      !isOpenAndClose &&
      /^</.test(line)
    ) {
      depth += 1
    }
  }
  return `${lines.join("\n")}\n`
}

function xmlErrorFromParser(errorNode: Element): string {
  const text = (errorNode.textContent ?? "Invalid XML").trim()
  const jsdom = text.match(/^(\d+):(\d+):\s*([\s\S]+)/)
  if (jsdom) {
    return `at line ${jsdom[1]}, column ${jsdom[2]}: ${jsdom[3].trim()}`
  }
  const chrome = text.match(
    /line\s+(\d+)\s*(?:at|,)\s*column\s+(\d+)[:\s]+([\s\S]+)/i
  )
  if (chrome) {
    return `at line ${chrome[1]}, column ${chrome[2]}: ${chrome[3].trim()}`
  }
  const generic = text.match(/line(?:\s*number)?\s+(\d+).*column\s+(\d+)/i)
  if (generic) {
    return `at line ${generic[1]}, column ${generic[2]}: ${text}`
  }
  return text
}

function formatXmlNode(node: Node, depth: number, indent: string): string {
  const pad = indent.repeat(depth)
  if (node.nodeType === Node.COMMENT_NODE) {
    return `${pad}<!--${node.textContent ?? ""}-->`
  }
  if (node.nodeType === Node.CDATA_SECTION_NODE) {
    return `${pad}<![CDATA[${node.textContent ?? ""}]]>`
  }
  if (node.nodeType === Node.PROCESSING_INSTRUCTION_NODE) {
    const pi = node as ProcessingInstruction
    const data = pi.data ? ` ${pi.data}` : ""
    return `${pad}<?${pi.target}${data}?>`
  }
  if (node.nodeType === Node.TEXT_NODE) {
    const text = (node.textContent ?? "").trim()
    return text ? `${pad}${escapeXmlText(text)}` : ""
  }
  if (node.nodeType !== Node.ELEMENT_NODE) return ""

  const el = node as Element
  const attrs = Array.from(el.attributes)
    .map((attr) => ` ${attr.name}="${escapeXmlAttr(attr.value)}"`)
    .join("")
  const children = Array.from(el.childNodes).filter((child) => {
    if (child.nodeType === Node.TEXT_NODE) {
      return (child.textContent ?? "").trim() !== ""
    }
    return (
      child.nodeType === Node.ELEMENT_NODE ||
      child.nodeType === Node.COMMENT_NODE ||
      child.nodeType === Node.CDATA_SECTION_NODE ||
      child.nodeType === Node.PROCESSING_INSTRUCTION_NODE
    )
  })
  if (children.length === 0) {
    return `${pad}<${el.tagName}${attrs}/>`
  }
  if (children.length === 1 && children[0].nodeType === Node.TEXT_NODE) {
    const text = escapeXmlText((children[0].textContent ?? "").trim())
    return `${pad}<${el.tagName}${attrs}>${text}</${el.tagName}>`
  }
  const inner = children
    .map((child) => formatXmlNode(child, depth + 1, indent))
    .filter(Boolean)
    .join("\n")
  return `${pad}<${el.tagName}${attrs}>\n${inner}\n${pad}</${el.tagName}>`
}

function formatXmlDocument(doc: Document, original: string): string {
  const parts: string[] = []
  const decl = original.match(/^\s*<\?xml\b[^?]*\?>/i)
  if (decl) parts.push(decl[0].trim())
  for (const node of Array.from(doc.childNodes)) {
    if (node.nodeType === Node.DOCUMENT_TYPE_NODE) continue
    const formatted = formatXmlNode(node, 0, "  ")
    if (formatted) parts.push(formatted)
  }
  return `${parts.join("\n")}\n`
}

export function formatXml(input: string): CodeFormatResult {
  const trimmed = input.trim()
  if (trimmed === "") return { ok: true, output: "" }
  if (typeof DOMParser === "undefined") {
    return { ok: true, output: indentXmlFallback(trimmed) }
  }
  const doc = new DOMParser().parseFromString(trimmed, "application/xml")
  const errorNode = doc.getElementsByTagName("parsererror")[0]
  if (errorNode) {
    return { ok: false, error: xmlErrorFromParser(errorNode) }
  }
  return { ok: true, output: formatXmlDocument(doc, trimmed) }
}

export function formatYamlView(input: string): CodeFormatResult {
  if (input.trim() === "") return { ok: true, output: "" }
  try {
    const value = load(input, { schema: JSON_SCHEMA, json: true })
    if (value === undefined) return { ok: true, output: "" }
    const output = dump(value, {
      indent: 2,
      lineWidth: -1,
      noRefs: true,
      schema: JSON_SCHEMA,
    })
    return { ok: true, output }
  } catch (error) {
    return { ok: false, error: yamlErrorMessage(error) }
  }
}

export function formatCode(
  input: string,
  kind: CodeFormatKind
): CodeFormatResult {
  return kind === "xml" ? formatXml(input) : formatYamlView(input)
}
