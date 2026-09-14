"use client"

import { useCallback, useEffect, useState } from "react"
import { Loader2 } from "lucide-react"
import { useTranslations } from "next-intl"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { ScrollArea } from "@/components/ui/scroll-area"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { toErrorMessage } from "@/lib/app-error"
import {
  getWikiSettings,
  wikiCancelJob,
  wikiCompileNow,
  wikiGetJob,
  wikiListJobs,
  wikiRetryJob,
} from "@/lib/api"
import { cn } from "@/lib/utils"
import {
  normalizeWikiList,
  normalizeWikiSettings,
  wikiJobKindKey,
  wikiSynthesizeEnabled,
  type WikiJob,
  type WikiJobStatus,
} from "@/lib/wiki-types"
import { WikiEmptyState } from "./wiki-shared"

const PAGE_SIZE = 50
const STATUS_FILTERS = [
  "all",
  "queued",
  "running",
  "succeeded",
  "failed",
  "cancelled",
] as const

function statusKey(
  status: string | null | undefined
): "queued" | "running" | "succeeded" | "failed" | "cancelled" | "unknown" {
  switch (status) {
    case "queued":
    case "running":
    case "succeeded":
    case "failed":
    case "cancelled":
      return status
    default:
      return "unknown"
  }
}

function statusVariant(
  status: string
): "secondary" | "destructive" | "outline" | "default" {
  if (status === "failed") return "destructive"
  if (status === "succeeded") return "default"
  if (status === "running") return "secondary"
  return "outline"
}

export function WikiJobsView() {
  const t = useTranslations("Wiki")
  const [loading, setLoading] = useState(true)
  const [loadingMore, setLoadingMore] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [jobs, setJobs] = useState<WikiJob[]>([])
  const [total, setTotal] = useState(0)
  const [status, setStatus] = useState<(typeof STATUS_FILTERS)[number]>("all")
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [detail, setDetail] = useState<WikiJob | null>(null)
  const [detailError, setDetailError] = useState<string | null>(null)
  const [detailLoading, setDetailLoading] = useState(false)
  const [actionBusy, setActionBusy] = useState(false)
  const [synthesizeEnabled, setSynthesizeEnabled] = useState(true)

  const load = useCallback(
    async (offset: number, nextStatus = status) => {
      const appending = offset > 0
      if (appending) setLoadingMore(true)
      else {
        setLoading(true)
        setError(null)
      }
      try {
        const page = normalizeWikiList<WikiJob>(
          await wikiListJobs({
            limit: PAGE_SIZE,
            offset,
            status:
              nextStatus === "all" ? undefined : (nextStatus as WikiJobStatus),
          })
        )
        setTotal(page.total)
        setJobs((current) =>
          appending ? [...current, ...page.items] : page.items
        )
      } catch (err) {
        if (!appending) {
          setJobs([])
          setTotal(0)
        }
        setError(t("loadFailed", { message: toErrorMessage(err) }))
      } finally {
        setLoading(false)
        setLoadingMore(false)
      }
    },
    [status, t]
  )

  useEffect(() => {
    load(0).catch(console.error)
  }, [load])

  useEffect(() => {
    getWikiSettings()
      .then((view) => {
        setSynthesizeEnabled(wikiSynthesizeEnabled(normalizeWikiSettings(view)))
      })
      .catch(console.error)
  }, [])

  const afterJobMutation = useCallback(
    async (job: WikiJob) => {
      await load(0)
      setSelectedId(job.id)
      setDetail(job)
    },
    [load]
  )

  const handleCompileNow = useCallback(async () => {
    setActionBusy(true)
    setDetailError(null)
    try {
      const job = await wikiCompileNow(crypto.randomUUID())
      await afterJobMutation(job)
    } catch (err) {
      setDetailError(t("loadFailed", { message: toErrorMessage(err) }))
    } finally {
      setActionBusy(false)
    }
  }, [afterJobMutation, t])

  const handleRetry = useCallback(async () => {
    if (!detail?.id) return
    setActionBusy(true)
    setDetailError(null)
    try {
      const job = await wikiRetryJob(detail.id)
      await afterJobMutation(job)
    } catch (err) {
      setDetailError(t("loadFailed", { message: toErrorMessage(err) }))
    } finally {
      setActionBusy(false)
    }
  }, [afterJobMutation, detail, t])

  const handleCancel = useCallback(async () => {
    if (!detail?.id) return
    setActionBusy(true)
    setDetailError(null)
    try {
      const job = await wikiCancelJob(detail.id)
      await afterJobMutation(job)
    } catch (err) {
      setDetailError(t("loadFailed", { message: toErrorMessage(err) }))
    } finally {
      setActionBusy(false)
    }
  }, [afterJobMutation, detail, t])

  const selectJob = useCallback(
    async (job: WikiJob) => {
      setSelectedId(job.id)
      setDetail(job)
      setDetailLoading(true)
      setDetailError(null)
      try {
        const next = await wikiGetJob(job.id)
        setDetail(next)
      } catch (err) {
        setDetailError(t("loadFailed", { message: toErrorMessage(err) }))
      } finally {
        setDetailLoading(false)
      }
    },
    [t]
  )

  return (
    <div className="flex h-full min-h-0 flex-col">
      <WikiEmptyState
        title={t("jobs.emptyTitle")}
        description={t("jobs.emptyDescription")}
      >
        <p className="text-xs leading-5 text-muted-foreground">
          {t("jobs.readOnlyHint")}
        </p>
      </WikiEmptyState>
      <div className="flex min-h-0 flex-1 flex-col border-t md:flex-row">
        <div className="flex min-h-0 w-full flex-col border-b md:w-80 md:border-b-0 md:border-r">
          <div className="flex items-center gap-2 px-3 py-2">
            <Select
              value={status}
              onValueChange={(value) => {
                setStatus(value as (typeof STATUS_FILTERS)[number])
              }}
            >
              <SelectTrigger className="h-8 min-w-0 flex-1">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {STATUS_FILTERS.map((value) => (
                  <SelectItem key={value} value={value}>
                    {value === "all"
                      ? t("jobs.allStatuses")
                      : t(`jobs.status.${value}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => load(0).catch(console.error)}
            >
              {t("refresh")}
            </Button>
            <Button
              type="button"
              size="sm"
              disabled={actionBusy || !synthesizeEnabled}
              title={synthesizeEnabled ? undefined : t("jobs.compileDisabled")}
              onClick={() => {
                handleCompileNow().catch(console.error)
              }}
            >
              {actionBusy ? t("jobs.compiling") : t("jobs.compileNow")}
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
                  onClick={() => load(0).catch(console.error)}
                >
                  {t("retry")}
                </Button>
              </div>
            ) : jobs.length === 0 ? (
              <p className="px-3 py-4 text-sm text-muted-foreground">
                {t("empty")}
              </p>
            ) : (
              <ul className="space-y-1 px-2 pb-3">
                {jobs.map((job) => {
                  const statusName = statusKey(job.status)
                  return (
                    <li key={job.id}>
                      <button
                        type="button"
                        className={cn(
                          "flex w-full flex-col gap-1 rounded-lg px-2 py-2 text-left hover:bg-muted/60",
                          selectedId === job.id && "bg-muted"
                        )}
                        onClick={() => {
                          selectJob(job).catch(console.error)
                        }}
                      >
                        <span className="flex items-center gap-2">
                          <Badge variant="outline">
                            {t(`jobs.kind.${wikiJobKindKey(job.kind)}`)}
                          </Badge>
                          <Badge variant={statusVariant(statusName)}>
                            {t(`jobs.status.${statusName}`)}
                          </Badge>
                        </span>
                        <span className="truncate font-mono text-xs text-muted-foreground">
                          {job.id}
                        </span>
                      </button>
                    </li>
                  )
                })}
              </ul>
            )}
            {jobs.length < total ? (
              <div className="px-3 pb-3">
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={loadingMore}
                  onClick={() => load(jobs.length).catch(console.error)}
                >
                  {loadingMore ? t("loading") : t("loadMore")}
                </Button>
              </div>
            ) : null}
          </ScrollArea>
        </div>
        <ScrollArea className="min-h-0 flex-1">
          <div className="space-y-3 p-4">
            {!detail ? (
              <p className="text-sm text-muted-foreground">
                {t("jobs.detailPlaceholder")}
              </p>
            ) : (
              <>
                <div className="flex flex-wrap items-center gap-2">
                  <Badge variant="outline">
                    {t(`jobs.kind.${wikiJobKindKey(detail.kind)}`)}
                  </Badge>
                  <Badge variant={statusVariant(statusKey(detail.status))}>
                    {t(`jobs.status.${statusKey(detail.status)}`)}
                  </Badge>
                  {detail.attempt != null ? (
                    <span className="text-xs text-muted-foreground">
                      {t("jobs.attempt", { n: detail.attempt })}
                    </span>
                  ) : null}
                </div>
                <p className="font-mono text-xs text-muted-foreground">
                  {detail.id}
                </p>
                {detailLoading ? (
                  <div className="flex items-center gap-2 text-sm text-muted-foreground">
                    <Loader2 className="size-4 animate-spin" />
                    {t("loading")}
                  </div>
                ) : null}
                {detailError ? (
                  <p className="text-sm text-destructive">{detailError}</p>
                ) : null}
                <div className="flex flex-wrap gap-2">
                  {statusKey(detail.status) === "failed" ? (
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      disabled={actionBusy}
                      onClick={() => {
                        handleRetry().catch(console.error)
                      }}
                    >
                      {t("jobs.retryJob")}
                    </Button>
                  ) : null}
                  {statusKey(detail.status) === "queued" ||
                  statusKey(detail.status) === "running" ? (
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      disabled={actionBusy}
                      onClick={() => {
                        handleCancel().catch(console.error)
                      }}
                    >
                      {t("jobs.cancelJob")}
                    </Button>
                  ) : null}
                </div>
                {detail.error ||
                detail.error_message ||
                detail.error_code ||
                detail.message ? (
                  <div className="space-y-1">
                    <p className="text-xs font-medium text-muted-foreground">
                      {t("jobs.error")}
                    </p>
                    <p className="text-sm">
                      {detail.error_code ? `${detail.error_code}: ` : ""}
                      {detail.error || detail.error_message || detail.message}
                    </p>
                  </div>
                ) : null}
                <dl className="grid gap-2 text-sm sm:grid-cols-2">
                  {detail.created_at ? (
                    <>
                      <dt className="text-muted-foreground">
                        {t("jobs.createdAt")}
                      </dt>
                      <dd>{detail.created_at}</dd>
                    </>
                  ) : null}
                  {detail.updated_at ? (
                    <>
                      <dt className="text-muted-foreground">
                        {t("jobs.updatedAt")}
                      </dt>
                      <dd>{detail.updated_at}</dd>
                    </>
                  ) : null}
                  {detail.started_at ? (
                    <>
                      <dt className="text-muted-foreground">
                        {t("jobs.startedAt")}
                      </dt>
                      <dd>{detail.started_at}</dd>
                    </>
                  ) : null}
                  {detail.finished_at ? (
                    <>
                      <dt className="text-muted-foreground">
                        {t("jobs.finishedAt")}
                      </dt>
                      <dd>{detail.finished_at}</dd>
                    </>
                  ) : null}
                </dl>
              </>
            )}
          </div>
        </ScrollArea>
      </div>
    </div>
  )
}
