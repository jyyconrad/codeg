/**
 * 个人 Wiki 的页面入口。主导航只有文件和任务：文件是库目录加 Markdown 阅读，任务是处理状态。
 * 旧的库、概览、工作、能力和资料地址都打开文件页，搜索不再改写到这些领域页。
 */
"use client"

import { useTranslations } from "next-intl"
import { WorkbenchPageTitle } from "@/components/workbench/workbench-page-title"
import { Button } from "@/components/ui/button"
import { WikiDataProvider, useWikiData, type WikiViewId } from "./wiki-data"
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
  const section = route.view === "jobs" ? "jobs" : "files"

  return (
    <div className="flex h-full min-h-0 min-w-0 flex-col">
      <header className="shrink-0 space-y-3 border-b px-3 py-3 md:px-4">
        <div className="flex flex-wrap items-center gap-2">
          <nav
            className="flex max-w-full gap-1 overflow-x-auto"
            aria-label={t("navigation")}
          >
            {(["files", "jobs"] as WikiViewId[]).map((view) => (
              <Button
                type="button"
                key={view}
                size="sm"
                variant={section === view ? "secondary" : "ghost"}
                aria-current={section === view ? "page" : undefined}
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
          </div>
        </div>
      </header>
      <main className="min-h-0 min-w-0 flex-1 overflow-hidden">
        {route.view === "jobs" ? (
          <WikiJobsView />
        ) : (
          <WikiNoteBrowser
            includeRaw
            emptyTitle={t("nav.files")}
            emptyDescription={t("fileBrowserHint")}
          />
        )}
      </main>
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
