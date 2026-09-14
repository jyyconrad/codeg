/**
 * 个人 Wiki 的页面入口，组织目录阅读、概览、工作、能力、资料和处理记录。
 * 通过 WikiDataProvider 共享路由与刷新，子视图分别承担阅读、导入和任务操作；保留内部文件弹窗。
 */
"use client"

import { useEffect, useState } from "react"
import Link from "next/link"
import { Search, Settings } from "lucide-react"
import { useTranslations } from "next-intl"
import { WorkbenchPageTitle } from "@/components/workbench/workbench-page-title"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog"
import { WikiDataProvider, useWikiData, type WikiViewId } from "./wiki-data"
import { WikiAllView } from "./wiki-all-view"
import { WikiLibraryView } from "./wiki-library-view"
import { WikiWorkView } from "./wiki-work-view"
import { WikiCapabilitiesView } from "./wiki-capabilities-view"
import { WikiSourcesView } from "./wiki-sources-view"
import { WikiJobsView } from "./wiki-jobs-view"
import { WikiImportButton } from "./wiki-import-dialog"
import { WikiNoteBrowser } from "./wiki-shared"

export type { WikiViewId } from "./wiki-data"
export function WikiPageTitle() {
  const t = useTranslations("Wiki")
  return <WorkbenchPageTitle title={t("title")} />
}
function WikiPageContent() {
  const t = useTranslations("Wiki.v2")
  const { route, navigate } = useWikiData()
  const [searchDraft, setSearchDraft] = useState({
    view: route.view,
    query: route.query,
    value: route.query,
  })
  const search =
    searchDraft.query === route.query && searchDraft.view === route.view
      ? searchDraft.value
      : route.query
  const [filesOpen, setFilesOpen] = useState(false)
  const [internal, setInternal] = useState(false)
  useEffect(() => {
    if (search === route.query || route.view === "jobs") return
    const timer = window.setTimeout(
      () =>
        navigate(
          {
            query: search,
            view: route.view === "sources" ? "sources" : "overview",
            path: null,
            source: null,
          },
          true
        ),
      300
    )
    return () => window.clearTimeout(timer)
  }, [search, route.query, route.view, navigate])

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-col">
      <header className="shrink-0 space-y-3 border-b px-3 py-3 md:px-4">
        <div className="flex flex-wrap items-center gap-2">
          <nav
            className="flex max-w-full gap-1 overflow-x-auto"
            aria-label={t("navigation")}
          >
            {(
              [
                "library",
                "overview",
                "work",
                "capabilities",
                "sources",
                "jobs",
              ] as WikiViewId[]
            ).map((view) => (
              <Button
                type="button"
                key={view}
                size="sm"
                variant={route.view === view ? "secondary" : "ghost"}
                aria-current={route.view === view ? "page" : undefined}
                onClick={() =>
                  navigate({
                    view,
                    path: null,
                    source: null,
                    job: null,
                    query: "",
                  })
                }
              >
                {t(`nav.${view}`)}
              </Button>
            ))}
          </nav>
          <div className="ms-auto flex flex-wrap gap-2">
            <WikiImportButton />
            <Button asChild variant="ghost" size="sm">
              <Link href="/settings/wiki?from=wiki">
                <Settings className="size-4" />
                {t("settings")}
              </Link>
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => setFilesOpen(true)}
            >
              {t("browseFiles")}
            </Button>
          </div>
        </div>
        {route.view !== "jobs" && (
          <form
            className="flex gap-2"
            onSubmit={(event) => {
              event.preventDefault()
              navigate({
                query: search,
                view: route.view === "sources" ? "sources" : "overview",
                path: null,
                source: null,
              })
            }}
          >
            <div className="relative min-w-0 flex-1">
              <Search className="pointer-events-none absolute start-3 top-2.5 size-4 text-muted-foreground" />
              <Input
                aria-label={t("search")}
                className="ps-9"
                value={search}
                onChange={(event) =>
                  setSearchDraft({
                    view: route.view,
                    query: route.query,
                    value: event.target.value,
                  })
                }
                placeholder={t("searchPlaceholder")}
              />
            </div>
            {route.view !== "sources" && (
              <Select
                value={route.scope}
                onValueChange={(value) =>
                  navigate(
                    {
                      scope: value as "all" | "notes" | "sources",
                    },
                    true
                  )
                }
              >
                <SelectTrigger
                  aria-label={t("searchScope")}
                  className="w-32"
                  size="sm"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent align="end">
                  {(["all", "notes", "sources"] as const).map((value) => (
                    <SelectItem key={value} value={value}>
                      {t(`scopes.${value}`)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            )}
            <Button type="submit" size="sm" className="h-9">
              {t("search")}
            </Button>
          </form>
        )}
      </header>
      <main className="min-h-0 min-w-0 flex-1 overflow-hidden">
        {route.view === "library" ? (
          <WikiLibraryView />
        ) : route.view === "overview" ? (
          <WikiAllView />
        ) : route.view === "work" ? (
          <WikiWorkView />
        ) : route.view === "capabilities" ? (
          <WikiCapabilitiesView />
        ) : route.view === "sources" ? (
          <WikiSourcesView />
        ) : (
          <WikiJobsView />
        )}
      </main>
      <Dialog open={filesOpen} onOpenChange={setFilesOpen}>
        <DialogContent className="flex h-[85dvh] max-w-6xl flex-col overflow-hidden">
          <DialogTitle>{t("browseFiles")}</DialogTitle>
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={internal}
              onChange={(event) => setInternal(event.target.checked)}
            />
            {t("showInternal")}
          </label>
          <div className="min-h-0 flex-1">
            <WikiNoteBrowser
              includeRaw={internal}
              emptyTitle={t("browseFiles")}
              emptyDescription={t("fileBrowserHint")}
            />
          </div>
        </DialogContent>
      </Dialog>
    </div>
  )
}
export function WikiPage() {
  return (
    <WikiDataProvider>
      <WikiPageContent />
    </WikiDataProvider>
  )
}
