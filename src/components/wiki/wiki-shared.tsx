/**
 * Wiki 阅读共用的空状态、Markdown 渲染、目录项和内部文件浏览器。
 * 链接解析使用 wiki-content；目录与原文读取使用共享查询，内部浏览选择不改变主阅读路由。
 */
"use client"

import { useCallback, useEffect, useState, type ReactNode } from "react"
import { ChevronRight, FileText, Folder, Loader2 } from "lucide-react"
import { useTranslations } from "next-intl"

import ReactMarkdown, { defaultUrlTransform } from "react-markdown"
import remarkGfm from "remark-gfm"
import { MarkdownLink } from "@/components/ai-elements/markdown-link"
import {
  prepareWikiMarkdown,
  resolveWikiLink,
  remarkWikiLinks,
  wikiFolderLandingPath,
} from "@/lib/wiki-content"
import { useWikiData, useWikiQuery } from "./wiki-data"

import { Button } from "@/components/ui/button"
import { ScrollArea } from "@/components/ui/scroll-area"
import { wikiVaultTree } from "@/lib/wiki-api"
import { cn } from "@/lib/utils"
import type { WikiVaultFile, WikiVaultTreeNode } from "@/lib/wiki-types"

export function WikiEmptyState({
  title,
  description,
  children,
}: {
  title: string
  description: string
  children?: ReactNode
}) {
  return (
    <div className="mx-auto flex max-w-2xl flex-col gap-3 px-4 py-8">
      <h2 className="text-sm font-semibold">{title}</h2>
      <p className="text-sm leading-6 text-muted-foreground">{description}</p>
      {children}
    </div>
  )
}

export function WikiMarkdownPreview({
  content,
  path = "",
  className,
  onOpenWikilink,
}: {
  content: string
  path?: string
  className?: string
  onOpenWikilink?: (path: string, anchor: string) => void
}) {
  const t = useTranslations("Wiki.v2")
  const prepared = prepareWikiMarkdown(content)
  return (
    <div
      className={cn(
        "min-w-0 break-words text-base leading-[1.7] [&_h1]:my-5 [&_h1]:text-2xl [&_h2]:mb-3 [&_h2]:mt-7 [&_h2]:text-xl [&_h3]:mb-2 [&_h3]:mt-5 [&_h3]:text-lg [&_h1]:font-semibold [&_h2]:font-semibold [&_h3]:font-semibold [&_p]:my-3 [&_ul]:my-3 [&_ul]:list-disc [&_ul]:ps-6 [&_ol]:my-3 [&_ol]:list-decimal [&_ol]:ps-6 [&_blockquote]:border-s-2 [&_blockquote]:ps-4 [&_blockquote]:text-muted-foreground [&_pre]:my-4 [&_pre]:max-w-full [&_pre]:overflow-x-auto [&_pre]:rounded-lg [&_pre]:bg-muted [&_pre]:p-4 [&_pre]:text-sm [&_code]:font-mono [&_a]:text-primary [&_a]:underline [&_th]:border [&_th]:p-2 [&_td]:border [&_td]:p-2 [&_img]:max-w-full [&_hr]:my-6",
        className
      )}
    >
      {prepared.warning && (
        <p role="status" className="text-sm text-amber-700 dark:text-amber-400">
          {t("formatWarning")}
        </p>
      )}
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkWikiLinks]}
        skipHtml
        urlTransform={(url) =>
          url.startsWith("wiki:") ? url : defaultUrlTransform(url)
        }
        components={{
          table: ({ children }) => (
            <div className="my-4 max-w-full overflow-x-auto">
              <table className="w-full border-collapse text-sm">
                {children}
              </table>
            </div>
          ),
          a: ({ href, children }) => {
            if (href && /^(https?:|mailto:|tel:)/i.test(href))
              return <MarkdownLink href={href}>{children}</MarkdownLink>
            const target = href ? resolveWikiLink(href, path) : null
            return (
              <a
                href={
                  target
                    ? `?wikiPath=${encodeURIComponent(target.path)}${target.anchor ? `#${encodeURIComponent(target.anchor)}` : ""}`
                    : undefined
                }
                aria-disabled={!target}
                onClick={(event) => {
                  event.preventDefault()
                  if (!target) return
                  if (
                    target.path === path &&
                    target.anchor &&
                    !/^l\d+(?:-l?\d+)?$/i.test(target.anchor)
                  )
                    document
                      .getElementById(target.anchor)
                      ?.scrollIntoView({ block: "start" })
                  else onOpenWikilink?.(target.path, target.anchor)
                }}
              >
                {children}
              </a>
            )
          },
        }}
      >
        {prepared.body}
      </ReactMarkdown>
    </div>
  )
}

export function VaultTreeItem({
  node,
  selected,
  onSelect,
  depth,
  loadChildren,
}: {
  node: WikiVaultTreeNode
  selected: string | null
  onSelect: (path: string) => void
  depth: number
  loadChildren: (path: string) => Promise<WikiVaultTreeNode[]>
}) {
  const t = useTranslations("Wiki")
  const { revision } = useWikiData()
  const isDir = node.is_dir
  const path = node.path
  const providedChildren = node.children
  const [expansion, setExpansion] = useState<{
    selected: string | null
    open: boolean
  } | null>(null)
  const open =
    expansion?.selected === selected
      ? expansion.open
      : !!selected && selected.startsWith(`${path}/`)
  const [fetched, setFetched] = useState<{
    revision: number
    children: WikiVaultTreeNode[]
  } | null>(null)

  const children = providedChildren ?? fetched?.children ?? []
  const loaded = providedChildren !== undefined || fetched !== null
  const landing = isDir
    ? wikiFolderLandingPath(path, loaded ? children : undefined)
    : null
  const folderCurrent = isDir && landing != null && selected === landing
  const kidsLoading =
    isDir &&
    open &&
    providedChildren === undefined &&
    fetched?.revision !== revision

  // 深层目录可能超出后端递归上限。自动展开和刷新都要补读，不能只在点击时加载。
  useEffect(() => {
    if (!isDir || !open || providedChildren !== undefined) return
    let stale = false
    loadChildren(path)
      .then((kids) => {
        if (!stale) setFetched({ revision, children: kids })
      })
      .catch(() => {
        if (!stale) setFetched({ revision, children: [] })
      })
    return () => {
      stale = true
    }
  }, [isDir, open, providedChildren, path, loadChildren, revision])

  const handleClick = () => {
    if (!isDir) {
      onSelect(path)
      return
    }
    setExpansion({ selected, open: true })
    if (landing) onSelect(landing)
  }

  const toggleFolder = (event: { stopPropagation: () => void }) => {
    event.stopPropagation()
    setExpansion({ selected, open: !open })
  }

  const label = node.title || node.name

  return (
    <li>
      <div
        className={cn(
          "flex w-full items-center gap-1 rounded-md px-2 py-1 text-sm hover:bg-muted/60",
          (selected === path || folderCurrent) && "bg-muted"
        )}
        style={{ paddingInlineStart: 8 + depth * 12 }}
      >
        {isDir ? (
          <button
            type="button"
            className="flex size-5 shrink-0 items-center justify-center rounded-sm text-muted-foreground hover:bg-muted"
            aria-expanded={open}
            aria-label={t(
              open
                ? "v2.library.collapseDirectory"
                : "v2.library.expandDirectory",
              { name: label }
            )}
            onClick={toggleFolder}
          >
            <ChevronRight
              className={cn(
                "size-3.5 transition-transform",
                open && "rotate-90"
              )}
            />
          </button>
        ) : (
          <FileText className="size-3.5 shrink-0 text-muted-foreground" />
        )}
        <button
          type="button"
          className="flex min-w-0 flex-1 items-center gap-1 text-left"
          onClick={handleClick}
          aria-expanded={isDir ? open : undefined}
          aria-current={
            (!isDir && selected === path) || folderCurrent ? "page" : undefined
          }
          title={path}
        >
          {isDir ? (
            <Folder className="size-3.5 shrink-0 text-muted-foreground" />
          ) : null}
          <span className="truncate">{label}</span>
        </button>
      </div>
      {isDir && open ? (
        kidsLoading && !loaded ? (
          <div
            className="flex items-center gap-2 py-1 text-xs text-muted-foreground"
            style={{ paddingInlineStart: 24 + depth * 12 }}
          >
            <Loader2 className="size-3 animate-spin" />
            {t("loading")}
          </div>
        ) : children.length > 0 ? (
          <ul>
            {children.map((child) => (
              <VaultTreeItem
                key={child.path}
                node={child}
                selected={selected}
                onSelect={onSelect}
                depth={depth + 1}
                loadChildren={loadChildren}
              />
            ))}
          </ul>
        ) : null
      ) : null}
    </li>
  )
}

export function WikiNoteBrowser({
  emptyTitle,
  emptyDescription,
  includeRaw = false,
}: {
  emptyTitle: string
  emptyDescription: string
  includeRaw?: boolean
}) {
  const t = useTranslations("Wiki")
  const { route } = useWikiData()
  // 文件弹窗保留自己的选择，关闭后不改变主 Wiki 页的阅读位置。
  const [selected, setSelected] = useState<string | null>(() => route.path)
  const tree = useWikiQuery<WikiVaultTreeNode[]>("wiki_vault_tree", {
    recursive: true,
    include_raw: includeRaw,
  })
  const preview = useWikiQuery<WikiVaultFile>(
    "wiki_vault_read",
    { path: selected },
    !!selected
  )
  const nodes = tree.data ?? []
  const loadChildren = useCallback(
    (path: string) => wikiVaultTree({ path, recursive: true, includeRaw }),
    [includeRaw]
  )

  return (
    <div className="flex h-full min-h-0 flex-col">
      <WikiEmptyState title={emptyTitle} description={emptyDescription} />
      <div className="flex min-h-0 flex-1 flex-col border-t md:flex-row">
        <div className="flex min-h-0 w-full flex-col border-b md:w-72 md:border-b-0 md:border-r">
          <div className="flex items-center justify-between gap-2 px-3 py-2">
            <p className="truncate text-xs font-medium text-muted-foreground">
              {t("treeTitle", { path: "/" })}
            </p>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={tree.refresh}
            >
              {t("refresh")}
            </Button>
          </div>
          <ScrollArea className="min-h-0 flex-1">
            {tree.loading && !tree.data ? (
              <div className="flex items-center gap-2 px-3 py-4 text-sm text-muted-foreground">
                <Loader2 className="size-4 animate-spin" />
                {t("loading")}
              </div>
            ) : tree.error ? (
              <div className="space-y-2 px-3 py-4 text-sm text-destructive">
                <p>{t("loadFailed", { message: tree.error })}</p>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={tree.refresh}
                >
                  {t("retry")}
                </Button>
              </div>
            ) : !nodes.length ? (
              <p className="px-3 py-4 text-sm text-muted-foreground">
                {t("empty")}
              </p>
            ) : (
              <ul className="px-1 pb-3">
                {nodes.map((node) => (
                  <VaultTreeItem
                    key={node.path}
                    node={node}
                    selected={selected}
                    onSelect={setSelected}
                    depth={0}
                    loadChildren={loadChildren}
                  />
                ))}
              </ul>
            )}
          </ScrollArea>
        </div>
        <ScrollArea className="min-h-0 flex-1">
          <div className="p-4">
            {selected ? (
              <p className="mb-3 truncate font-mono text-xs text-muted-foreground">
                {selected}
              </p>
            ) : (
              <p className="text-sm text-muted-foreground">
                {t("note.preview")}
              </p>
            )}
            {preview.loading ? (
              <div className="flex items-center gap-2 text-sm text-muted-foreground">
                <Loader2 className="size-4 animate-spin" />
                {t("loading")}
              </div>
            ) : preview.error ? (
              <p className="text-sm text-destructive">
                {t("loadFailed", { message: preview.error })}
              </p>
            ) : selected && preview.data ? (
              <WikiMarkdownPreview
                content={preview.data.content}
                path={selected}
                onOpenWikilink={setSelected}
              />
            ) : null}
          </div>
        </ScrollArea>
      </div>
    </div>
  )
}
