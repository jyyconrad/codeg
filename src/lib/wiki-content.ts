/**
 * Wiki Markdown 阅读的纯转换工具：隐藏合法元数据、解析站内链接、标题和原文定位。
 * 由笔记及素材阅读器调用；只处理显示与导航，不验证生成结论，也不读取文件或发起请求。
 */
import { load as loadYaml } from "js-yaml"
import type { Root, PhrasingContent, RootContent } from "mdast"
import { visit } from "unist-util-visit"

/** 元数据损坏或未闭合时保留原文，不能为了隐藏 YAML 吞掉用户内容。 */
export function prepareWikiMarkdown(source: string): {
  body: string
  warning: boolean
} {
  const text = source.replace(/^\uFEFF/, "")
  if (!/^---\r?\n/.test(text)) return { body: text, warning: false }
  const end = /^---\s*$/gm
  end.lastIndex = text.indexOf("\n") + 1
  const match = end.exec(text)
  if (!match) return { body: text, warning: true }
  try {
    const metadata = loadYaml(text.slice(text.indexOf("\n") + 1, match.index))
    if (!metadata || typeof metadata !== "object" || Array.isArray(metadata))
      return { body: text, warning: true }
    return {
      body: text.slice(match.index + match[0].length).replace(/^\r?\n/, ""),
      warning: false,
    }
  } catch {
    return { body: text, warning: true }
  }
}

export function wikiHeadingId(title: string): string {
  return title
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s_-]/gu, "")
    .replace(/\s/g, "-")
}

export function resolveWikiLink(
  href: string,
  currentPath: string
): { path: string; anchor: string } | null {
  let target: string
  let anchor: string
  try {
    const hash = href.indexOf("#")
    target = decodeURIComponent(hash < 0 ? href : href.slice(0, hash))
    anchor =
      hash < 0 ? "" : wikiHeadingId(decodeURIComponent(href.slice(hash + 1)))
  } catch {
    return null
  }
  const library = target.startsWith("wiki:")
  if (library) target = target.slice(5)
  if (
    /^[a-z][a-z\d+.-]*:/i.test(target) ||
    target.startsWith("/") ||
    /[\\\u0000-\u001f?]/.test(target)
  )
    return null
  const relative = target
  if (!relative) return currentPath ? { path: currentPath, anchor } : null
  const parts = library ? [] : currentPath.split("/").slice(0, -1)
  for (const part of relative.split("/")) {
    if (!part || part === ".") continue
    if (part === "..") {
      if (!parts.length) return null
      parts.pop()
    } else parts.push(part)
  }
  if (!parts.length) return null
  const path = parts.join("/")
  return { path: /\.[^/]+$/.test(path) ? path : `${path}.md`, anchor }
}

/** 仅转换 Markdown 文本节点；代码和已有链接内的双括号必须保持原样。 */
export function remarkWikiLinks() {
  return (tree: Root) => {
    visit(tree, "text", (node, index, parent) => {
      if (
        index == null ||
        !parent ||
        parent.type === "link" ||
        parent.type === "linkReference"
      )
        return
      const regex = /\[\[([^\]\n]+)\]\]/g
      const children: PhrasingContent[] = []
      let cursor = 0
      for (const match of node.value.matchAll(regex)) {
        const start = match.index!
        if (start > cursor)
          children.push({
            type: "text",
            value: node.value.slice(cursor, start),
          })
        const [target, ...label] = match[1].split("|")
        children.push({
          type: "link",
          url: `wiki:${target.trim()}`,
          children: [
            { type: "text", value: label.join("|").trim() || target.trim() },
          ],
        })
        cursor = start + match[0].length
      }
      if (!cursor) return
      if (cursor < node.value.length)
        children.push({ type: "text", value: node.value.slice(cursor) })
      ;(parent.children as RootContent[]).splice(index, 1, ...children)
      return index + children.length
    })
    const ids = new Map<string, number>()
    visit(tree, "heading", (node) => {
      let label = ""
      visit(node, (child) => {
        if (child.type === "text" || child.type === "inlineCode")
          label += child.value
      })
      const base = wikiHeadingId(label)
      const duplicate = ids.get(base) ?? 0
      ids.set(base, duplicate + 1)
      node.data = {
        ...node.data,
        hProperties: {
          ...node.data?.hProperties,
          id: duplicate ? `${base}-${duplicate}` : base,
        },
      }
    })
  }
}

/** 来源行号对应未经隐藏元数据的原文；这里只做阅读定位。 */
export function wikiSourceLineExcerpt(
  source: string,
  hash: string
): { start: number; end: number; text: string } | null {
  const match = /^#?l(\d+)(?:-l?(\d+))?$/i.exec(hash)
  if (!match) return null
  const start = Number(match[1]),
    end = Number(match[2] ?? match[1])
  const lines = source.split(/\r?\n/)
  if (start < 1 || end < start || start > lines.length) return null
  return {
    start,
    end: Math.min(end, lines.length),
    text: lines.slice(start - 1, end).join("\n"),
  }
}

const FOLDER_LANDING_NAMES = ["index.md", "README.md", "readme.md"]

/** GitHub-style folder landing: index.md, then README.md. */
export function wikiFolderLandingPath(
  path: string,
  children?: { name: string; path: string; is_dir: boolean }[]
): string | null {
  if (!path || !children) return null
  for (const name of FOLDER_LANDING_NAMES) {
    const hit = children.find((child) => !child.is_dir && child.name === name)
    if (hit) return hit.path
  }
  return null
}

/** 阅读器已经显示文档标题，正文中相同的首个标题不重复展示。 */
export function wikiBodyForReading(body: string, title: string): string {
  const heading = /^\s*# ([^\n]+)\r?\n/.exec(body)
  return heading && heading[1].trim() === title.trim()
    ? body.slice(heading[0].length).trimStart()
    : body
}
