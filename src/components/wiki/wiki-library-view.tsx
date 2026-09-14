/**
 * 目录式 Wiki 阅读入口：获取后端维护的 Markdown 首页、目录与实际仓库位置。
 * 复用目录项和正文阅读器，库内跳转保持阅读上下文，并提供 Obsidian 文件夹路径说明。
 */
"use client"

import { useState } from "react"
import { BookOpen, Check, Copy, FolderTree, Loader2 } from "lucide-react"
import { useTranslations } from "next-intl"
import { Button } from "@/components/ui/button"
import { isLocalDesktop } from "@/lib/platform"
import { copyTextToClipboard } from "@/lib/utils"
import type { WikiLibrary, WikiVaultTreeNode } from "@/lib/wiki-types"
import { wikiVaultTree } from "@/lib/wiki-api"
import { useWikiData, useWikiQuery } from "./wiki-data"
import { WikiNoteReader } from "./wiki-note-reader"
import { VaultTreeItem } from "./wiki-shared"

async function loadChildren(path: string): Promise<WikiVaultTreeNode[]> {
  return wikiVaultTree({ path, recursive: true })
}

export function WikiLibraryView() {
  const t = useTranslations("Wiki.v2")
  const { route, navigate } = useWikiData()
  const library = useWikiQuery<WikiLibrary>("wiki_refresh_library")
  const [directoryOpen, setDirectoryOpen] = useState(false)
  const [copiedPath, setCopiedPath] = useState<string | null>(null)
  const [copyFailed, setCopyFailed] = useState(false)
  const homePath = library.data?.home_path ?? "index.md"
  const selected = route.path ?? homePath
  const select = (path: string) => {
    navigate({ view: "library", path, query: "" })
    setDirectoryOpen(false)
  }

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-col md:flex-row">
      <div className="shrink-0 border-b px-3 py-2 md:hidden">
        <Button
          size="sm"
          variant="ghost"
          aria-expanded={directoryOpen}
          aria-controls="wiki-library-directory"
          onClick={() => setDirectoryOpen((value) => !value)}
        >
          <FolderTree className="size-4" />
          {t("library.directory")}
        </Button>
      </div>
      <aside
        id="wiki-library-directory"
        className={`min-h-0 min-w-0 shrink-0 flex-col border-e md:flex md:w-72 md:flex-none ${directoryOpen ? "flex flex-1" : "hidden"}`}
      >
        <div className="flex items-center justify-between gap-2 border-b px-3 py-2">
          <h2 className="text-sm font-semibold">{t("library.directory")}</h2>
          <Button
            size="sm"
            variant="ghost"
            onClick={library.refresh}
            disabled={library.loading}
          >
            {library.loading && <Loader2 className="size-3.5 animate-spin" />}
            {t("refresh")}
          </Button>
        </div>
        <nav
          aria-label={t("library.directory")}
          className="min-h-0 flex-1 overflow-y-auto p-2"
        >
          <Button
            variant="ghost"
            size="sm"
            className="mb-2 w-full justify-start"
            onClick={() => select(homePath)}
          >
            <BookOpen className="size-4" />
            {t("library.home")}
          </Button>
          {!library.data && library.loading && (
            <p role="status" className="p-2 text-sm text-muted-foreground">
              {t("loading")}
            </p>
          )}
          {library.data && (
            <ul className="space-y-0.5">
              {library.data.tree.map((node) => (
                <VaultTreeItem
                  key={node.path}
                  node={node}
                  selected={selected}
                  onSelect={select}
                  depth={0}
                  loadChildren={loadChildren}
                />
              ))}
            </ul>
          )}
        </nav>
        {library.data && (
          <div className="shrink-0 space-y-2 border-t p-3 text-xs leading-5 text-muted-foreground">
            <div className="flex items-center justify-between gap-2">
              <p className="font-medium text-foreground">
                {t("library.location")}
              </p>
              <Button
                size="sm"
                variant="ghost"
                aria-label={t("library.copyPath")}
                onClick={async () => {
                  const path = library.data!.vault_path
                  const copied = await copyTextToClipboard(path)
                  setCopiedPath(copied ? path : null)
                  setCopyFailed(!copied)
                }}
              >
                {copiedPath === library.data.vault_path ? (
                  <Check className="size-3.5" />
                ) : (
                  <Copy className="size-3.5" />
                )}
              </Button>
            </div>
            <p className="break-all font-mono select-text">
              {library.data.vault_path}
            </p>
            <p>{t("library.obsidianHint")}</p>
            {!isLocalDesktop() && <p>{t("library.serverHint")}</p>}
            {copyFailed && <p role="alert">{t("library.copyFailed")}</p>}
            {copiedPath === library.data.vault_path && (
              <p role="status">{t("library.copied")}</p>
            )}
          </div>
        )}
      </aside>
      <div
        className={`min-h-0 min-w-0 flex-1 overflow-y-auto ${directoryOpen ? "hidden md:block" : ""}`}
      >
        {library.error && (
          <div role="alert" className="m-4 rounded-lg border p-4 text-sm">
            <p>{library.error}</p>
            <Button
              size="sm"
              variant="outline"
              className="mt-2"
              onClick={library.refresh}
            >
              {t("retry")}
            </Button>
          </div>
        )}
        {!!library.data?.warnings.length && (
          <details className="mx-4 mt-4 rounded-lg border p-3 text-sm">
            <summary className="cursor-pointer">
              {t("library.warnings")}
            </summary>
            <ul className="mt-2 list-disc space-y-1 break-words ps-5 text-muted-foreground">
              {library.data.warnings.map((warning, index) => (
                <li key={index}>{warning}</li>
              ))}
            </ul>
          </details>
        )}
        {library.data ? (
          <WikiNoteReader
            key={`${library.data.vault_path}:${selected}`}
            path={selected}
            homePath={homePath}
            refreshRevision={library.generation}
          />
        ) : library.loading ? (
          <p role="status" className="p-6 text-sm text-muted-foreground">
            {t("loading")}
          </p>
        ) : null}
      </div>
    </div>
  )
}
