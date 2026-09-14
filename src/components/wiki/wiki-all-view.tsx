/**
 * Wiki 概览与检索入口，展示近期笔记、资料数量和待处理状态。
 * 汇总数据来自共享读取模型；搜索交给笔记视图，整理和导入使用各自业务操作入口。
 */
"use client"

import Link from "next/link"
import { useTranslations } from "next-intl"
import { Button } from "@/components/ui/button"
import { useActiveFolder } from "@/contexts/active-folder-context"
import { useTabActions } from "@/stores/tab-store"
import { useOptionalWorkbenchRoute } from "@/contexts/workbench-route-context"
import { useWikiData } from "./wiki-data"
import { WikiNoteCard } from "./wiki-note-reader"
import { WikiNotesView } from "./wiki-notes-view"
import { WikiCompileButton } from "./wiki-jobs-view"
import { WikiImportButton } from "./wiki-import-dialog"

export function WikiAllView() {
  const t = useTranslations("Wiki.v2")
  const { overview, overviewError, route, navigate, invalidate, setJobFilter } =
    useWikiData()
  const workbench = useOptionalWorkbenchRoute()
  const { activeFolder } = useActiveFolder()
  const { openNewConversationTab, openChatModeTab } = useTabActions()
  if (route.query || route.path) return <WikiNotesView view="overview" />
  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-6xl space-y-6 p-4 md:p-6">
        {overviewError && (
          <div
            role="alert"
            className="rounded-lg border border-destructive/30 p-4 text-sm"
          >
            {overviewError}
            <Button
              size="sm"
              variant="outline"
              className="ms-3"
              onClick={invalidate}
            >
              {t("retry")}
            </Button>
          </div>
        )}
        {!overview && !overviewError && <p role="status">{t("loading")}</p>}
        {overview && (
          <>
            {!overview.enabled && (
              <section className="rounded-xl border bg-muted/30 p-6">
                <h1 className="text-xl font-semibold">
                  {overview.note_count ? t("paused") : t("welcome")}
                </h1>
                <p className="my-3 max-w-2xl text-sm leading-6 text-muted-foreground">
                  {overview.note_count ? t("pausedHint") : t("welcomeHint")}
                </p>
                <Button asChild>
                  <Link href="/settings/wiki?from=wiki">
                    {overview.note_count ? t("resume") : t("enable")}
                  </Link>
                </Button>
              </section>
            )}
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div>
                <h2 className="text-lg font-semibold">{t("recent")}</h2>
                <p className="mt-1 text-sm text-muted-foreground">
                  {overview.active_job_count
                    ? t("processingCount", { count: overview.active_job_count })
                    : overview.pending_memory_count
                      ? t("pendingCount", {
                          count: overview.pending_memory_count,
                        })
                      : t("upToDate")}
                </p>
              </div>
              <WikiCompileButton />
            </div>
            <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
              {(
                [
                  ["readable", overview.note_count, "work"],
                  ["sourceCount", overview.source_count, "sources"],
                  [
                    "pending",
                    overview.active_job_count + overview.pending_memory_count,
                    "jobs",
                  ],
                  ["needsAttention", overview.failed_job_count, "jobs"],
                ] as const
              ).map(([key, count, view]) => (
                <button
                  type="button"
                  key={key}
                  className="rounded-xl border p-4 text-start hover:bg-muted/50"
                  onClick={() => {
                    if (key === "needsAttention") setJobFilter("failed")
                    else if (key === "pending") setJobFilter("active")
                    navigate({ view, path: null, job: null, source: null })
                  }}
                >
                  <span className="block text-sm text-muted-foreground">
                    {t(key)}
                  </span>
                  <span className="mt-2 block text-2xl font-semibold">
                    {count}
                  </span>
                  {key === "pending" && (
                    <span className="mt-1 block text-xs text-muted-foreground">
                      {t("pendingDetail", {
                        jobs: overview.active_job_count,
                        notes: overview.pending_memory_count,
                      })}
                    </span>
                  )}
                </button>
              ))}
            </div>
            {overview.enabled && !overview.note_count && (
              <section className="rounded-xl border p-6">
                <h2 className="font-semibold">{t("noNotes")}</h2>
                <p className="my-3 text-sm leading-6 text-muted-foreground">
                  {t("noNotesHint")}
                </p>
                <div className="flex flex-wrap gap-2">
                  {workbench && (
                    <Button
                      onClick={() => {
                        workbench.openConversations()
                        if (activeFolder)
                          openNewConversationTab(
                            activeFolder.id,
                            activeFolder.path
                          )
                        else openChatModeTab()
                      }}
                    >
                      {t("startConversation")}
                    </Button>
                  )}
                  <WikiImportButton />
                </div>
              </section>
            )}
            <div className="grid gap-3 lg:grid-cols-2">
              {overview.recent_notes.map((note) => (
                <div
                  key={`${note.note_id}:${note.path}`}
                  className="rounded-xl border"
                >
                  <WikiNoteCard
                    note={note}
                    onClick={() =>
                      navigate({
                        view:
                          note.type === "capability" ? "capabilities" : "work",
                        path: note.path,
                        source: null,
                      })
                    }
                  />
                </div>
              ))}
            </div>
          </>
        )}
      </div>
    </div>
  )
}
