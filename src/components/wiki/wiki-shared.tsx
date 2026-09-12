"use client"

import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react"
import { ChevronRight, FileText, Folder, Loader2 } from "lucide-react"
import { useTranslations } from "next-intl"

import { Button } from "@/components/ui/button"
import { ScrollArea } from "@/components/ui/scroll-area"
import { toErrorMessage } from "@/lib/app-error"
import { wikiVaultRead, wikiVaultTree } from "@/lib/api"
import { cn } from "@/lib/utils"
import {
  normalizeWikiVaultTree,
  vaultNodesUnderPrefix,
  wikiNodeIsDir,
  wikiNodeName,
  wikiNodePath,
  wikiVaultReadContent,
  type WikiVaultTreeNode,
} from "@/lib/wiki-types"

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
  className,
}: {
  content: string
  className?: string
}) {
  return (
    <pre
      className={cn(
        "whitespace-pre-wrap break-words font-mono text-xs leading-5",
        className
      )}
    >
      {content}
    </pre>
  )
}

function readWikiPathParam(): string | null {
  if (typeof window === "undefined") return null
  return new URLSearchParams(window.location.search).get("wikiPath")
}

function writeWikiPathParam(path: string | null) {
  if (typeof window === "undefined") return
  const url = new URL(window.location.href)
  if (path) url.searchParams.set("wikiPath", path)
  else url.searchParams.delete("wikiPath")
  window.history.replaceState(
    null,
    "",
    `${url.pathname}${url.search}${url.hash}`
  )
}

function VaultTreeItem({
  node,
  selected,
  onSelect,
  depth,
}: {
  node: WikiVaultTreeNode
  selected: string | null
  onSelect: (path: string) => void
  depth: number
}) {
  const isDir = wikiNodeIsDir(node)
  const path = wikiNodePath(node)
  const [open, setOpen] = useState(depth < 1)
  const children = node.children ?? []

  return (
    <li>
      <button
        type="button"
        className={cn(
          "flex w-full items-center gap-1 rounded-md px-2 py-1 text-left text-sm hover:bg-muted/60",
          selected === path && "bg-muted"
        )}
        style={{ paddingInlineStart: 8 + depth * 12 }}
        onClick={() => {
          if (isDir) setOpen((value) => !value)
          else onSelect(path)
        }}
      >
        {isDir ? (
          <ChevronRight
            className={cn(
              "size-3.5 shrink-0 text-muted-foreground transition-transform",
              open && "rotate-90"
            )}
          />
        ) : (
          <FileText className="size-3.5 shrink-0 text-muted-foreground" />
        )}
        {isDir ? (
          <Folder className="size-3.5 shrink-0 text-muted-foreground" />
        ) : null}
        <span className="truncate">{wikiNodeName(node)}</span>
      </button>
      {isDir && open && children.length > 0 ? (
        <ul>
          {children.map((child) => (
            <VaultTreeItem
              key={wikiNodePath(child)}
              node={child}
              selected={selected}
              onSelect={onSelect}
              depth={depth + 1}
            />
          ))}
        </ul>
      ) : null}
    </li>
  )
}

export function WikiNoteBrowser({
  prefix,
  emptyTitle,
  emptyDescription,
  extras,
}: {
  prefix: string
  emptyTitle: string
  emptyDescription: string
  extras?: ReactNode
}) {
  const t = useTranslations("Wiki")
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [nodes, setNodes] = useState<WikiVaultTreeNode[]>([])
  const [selected, setSelected] = useState<string | null>(null)
  const [preview, setPreview] = useState<string>("")
  const [previewError, setPreviewError] = useState<string | null>(null)
  const [previewLoading, setPreviewLoading] = useState(false)

  const loadTree = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      const payload = await wikiVaultTree()
      setNodes(vaultNodesUnderPrefix(normalizeWikiVaultTree(payload), prefix))
    } catch (err) {
      setNodes([])
      setError(t("loadFailed", { message: toErrorMessage(err) }))
    } finally {
      setLoading(false)
    }
  }, [prefix, t])

  useEffect(() => {
    loadTree().catch(console.error)
  }, [loadTree])

  useEffect(() => {
    const initial = readWikiPathParam()
    if (initial && (initial === prefix || initial.startsWith(`${prefix}/`))) {
      setSelected(initial)
    }
  }, [prefix])

  const selectPath = useCallback((path: string) => {
    setSelected(path)
    writeWikiPathParam(path)
  }, [])

  useEffect(() => {
    if (!selected) {
      setPreview("")
      setPreviewError(null)
      return
    }
    let cancelled = false
    setPreviewLoading(true)
    setPreviewError(null)
    wikiVaultRead(selected)
      .then((payload) => {
        if (cancelled) return
        setPreview(wikiVaultReadContent(payload))
      })
      .catch((err) => {
        if (cancelled) return
        setPreview("")
        setPreviewError(t("loadFailed", { message: toErrorMessage(err) }))
      })
      .finally(() => {
        if (!cancelled) setPreviewLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [selected, t])

  const hasNotes = nodes.length > 0
  const treeTitle = useMemo(() => `${prefix}/`, [prefix])

  return (
    <div className="flex h-full min-h-0 flex-col">
      <WikiEmptyState title={emptyTitle} description={emptyDescription}>
        {extras}
      </WikiEmptyState>
      <div className="flex min-h-0 flex-1 flex-col border-t md:flex-row">
        <div className="flex min-h-0 w-full flex-col border-b md:w-72 md:border-b-0 md:border-r">
          <div className="flex items-center justify-between gap-2 px-3 py-2">
            <p className="truncate text-xs font-medium text-muted-foreground">
              {t("treeTitle", { path: treeTitle })}
            </p>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => loadTree().catch(console.error)}
            >
              {t("refresh")}
            </Button>
          </div>
          <ScrollArea className="min-h-0 flex-1">
            {loading ? (
              <div className="flex items-center gap-2 px-3 py-4 text-sm text-muted-foreground">
                <Loader2 className="size-4 animate-spin" />
                {t("loading")}
              </div>
            ) : error ? (
              <div className="space-y-2 px-3 py-4 text-sm text-destructive">
                <p>{error}</p>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={() => loadTree().catch(console.error)}
                >
                  {t("retry")}
                </Button>
              </div>
            ) : !hasNotes ? (
              <p className="px-3 py-4 text-sm text-muted-foreground">
                {t("empty")}
              </p>
            ) : (
              <ul className="px-1 pb-3">
                {nodes.map((node) => (
                  <VaultTreeItem
                    key={wikiNodePath(node)}
                    node={node}
                    selected={selected}
                    onSelect={selectPath}
                    depth={0}
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
            {previewLoading ? (
              <div className="flex items-center gap-2 text-sm text-muted-foreground">
                <Loader2 className="size-4 animate-spin" />
                {t("loading")}
              </div>
            ) : previewError ? (
              <p className="text-sm text-destructive">{previewError}</p>
            ) : (
              <WikiMarkdownPreview content={preview} />
            )}
          </div>
        </ScrollArea>
      </div>
    </div>
  )
}
