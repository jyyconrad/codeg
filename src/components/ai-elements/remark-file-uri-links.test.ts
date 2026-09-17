import { describe, expect, it } from "vitest"
import {
  remarkAutolinkInlineFilePaths,
  remarkRewriteFileUriLinks,
} from "./remark-file-uri-links"

// Minimal mdast node shapes for the transform.
type Node = {
  type: string
  url?: string
  identifier?: string
  children?: Node[]
}

function linkTree(url: string): Node {
  return {
    type: "root",
    children: [
      {
        type: "paragraph",
        children: [{ type: "link", url, children: [{ type: "text" }] }],
      },
    ],
  }
}

function firstLinkUrl(tree: Node): string | undefined {
  let found: string | undefined
  const walk = (n: Node) => {
    if (n.type === "link") found = n.url
    n.children?.forEach(walk)
  }
  walk(tree)
  return found
}

function rewrite(url: string): string | undefined {
  const tree = linkTree(url)
  remarkRewriteFileUriLinks()(tree)
  return firstLinkUrl(tree)
}

describe("remarkRewriteFileUriLinks", () => {
  it("rewrites a POSIX file:// URI to a bare local path", () => {
    expect(rewrite("file:///Users/a/b.ts")).toBe("/Users/a/b.ts")
  })

  it("keeps the leading slash before a Windows drive letter (sanitize-safe)", () => {
    // A bare `C:/…` would make rehype-sanitize read `C:` as a URL protocol and
    // strip the href (→ harden's "[blocked]"); `/C:/…` keeps `C:` out of
    // protocol position. Downstream link-safety strips the slash before opening.
    expect(rewrite("file:///C:/x/y.ts")).toBe("/C:/x/y.ts")
  })

  it("prefixes a slash onto a bare Windows drive path (forward slashes)", () => {
    expect(rewrite("E:/Desktop/docs/G.docx")).toBe("/E:/Desktop/docs/G.docx")
  })

  it("prefixes a slash onto a bare Windows drive path (backslashes)", () => {
    expect(rewrite("C:\\Users\\a\\b.docx")).toBe("/C:\\Users\\a\\b.docx")
  })

  it("prefixes a slash onto a Chinese/encoded bare Windows drive path", () => {
    expect(rewrite("E:/桌面/使用手册/G手册.docx")).toBe(
      "/E:/桌面/使用手册/G手册.docx"
    )
    expect(rewrite("E:/My%20Docs/%E6%89%8B%E5%86%8C.docx")).toBe(
      "/E:/My%20Docs/%E6%89%8B%E5%86%8C.docx"
    )
  })

  it("leaves a bare relative path untouched (not a drive path)", () => {
    // `C:` needs a following slash to be a drive path; `src/main.rs` and a
    // schemeless relative path stay as-is (not openable — existing behavior).
    expect(rewrite("src/main.rs")).toBe("src/main.rs")
    expect(rewrite("notes.md")).toBe("notes.md")
  })

  it("emits a UNC file:// URI as a backslash UNC path (unambiguously local)", () => {
    // //server/share would be indistinguishable from a protocol-relative
    // web url downstream; the backslash form tags it as a local file.
    expect(rewrite("file://server/share/doc.md")).toBe(
      "\\\\server\\share\\doc.md"
    )
  })

  it("preserves fragments on rewritten links", () => {
    expect(rewrite("file:///Users/a/b.ts#L12")).toBe("/Users/a/b.ts#L12")
  })

  it("leaves non-file URLs untouched", () => {
    expect(rewrite("https://example.com/x")).toBe("https://example.com/x")
  })
})

describe("remarkAutolinkInlineFilePaths", () => {
  function treeWithInlineCode(value: string): Node {
    return {
      type: "root",
      children: [
        {
          type: "paragraph",
          children: [{ type: "inlineCode", value } as Node],
        },
      ],
    }
  }

  function firstChildOfParagraph(tree: Node): Node | undefined {
    const para = tree.children?.[0]
    return para?.children?.[0]
  }

  it("turns an absolute POSIX path in inline code into a file link", () => {
    const tree = treeWithInlineCode(
      "/Users/jiangyayun/develop/code/work_code/switchgear/docs/使用手册.docx"
    )
    remarkAutolinkInlineFilePaths()(tree)
    const node = firstChildOfParagraph(tree)
    expect(node?.type).toBe("link")
    expect(node?.url).toBe(
      "/Users/jiangyayun/develop/code/work_code/switchgear/docs/使用手册.docx"
    )
  })

  it("rewrites file:// inline code to a sanitize-safe path href", () => {
    const tree = treeWithInlineCode("file:///Users/a/手册.docx")
    remarkAutolinkInlineFilePaths()(tree)
    expect(firstChildOfParagraph(tree)?.url).toBe(
      "/Users/a/%E6%89%8B%E5%86%8C.docx"
    )
  })

  it("turns a workspace-relative path in inline code into a file link", () => {
    const tree = treeWithInlineCode("docs/使用手册.docx")
    remarkAutolinkInlineFilePaths()(tree)
    const node = firstChildOfParagraph(tree)
    expect(node?.type).toBe("link")
    expect(node?.url).toBe("docs/使用手册.docx")
  })

  it("turns a document filename in inline code into a file link", () => {
    const tree = treeWithInlineCode("使用手册.docx")
    remarkAutolinkInlineFilePaths()(tree)
    expect(firstChildOfParagraph(tree)?.type).toBe("link")
  })

  it("does not promote extension-less directories or bare identifiers", () => {
    for (const value of ["/usr/bin", "/api/users", "app.ts", "foo"]) {
      const tree = treeWithInlineCode(value)
      remarkAutolinkInlineFilePaths()(tree)
      expect(firstChildOfParagraph(tree)?.type).toBe("inlineCode")
    }
  })

  it("leaves inline code inside an existing link alone", () => {
    const tree: Node = {
      type: "root",
      children: [
        {
          type: "paragraph",
          children: [
            {
              type: "link",
              url: "https://example.com",
              children: [
                {
                  type: "inlineCode",
                  value: "/Users/a/b.docx",
                } as Node,
              ],
            },
          ],
        },
      ],
    }
    remarkAutolinkInlineFilePaths()(tree)
    const inner = tree.children?.[0]?.children?.[0]?.children?.[0]
    expect(inner?.type).toBe("inlineCode")
  })
})
