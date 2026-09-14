/**
 * Wiki 处理记录与手动整理入口，展示任务状态、尝试历史和可打开的产物。
 * 共享读取模型负责查询，专用 API 负责发起、重试和取消任务；本页不执行内容生成。
 */
"use client"

import { useState } from "react"
import Link from "next/link"
import { useTranslations } from "next-intl"
import { ArrowLeft } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { wikiCancelJob, wikiCompileNow, wikiRetryJob } from "@/lib/wiki-api"
import { toErrorMessage } from "@/lib/app-error"
import {
  wikiJobErrorMessage,
  wikiJobKindKey,
  type WikiJob,
  type WikiListPage,
  type WikiSettingsView,
} from "@/lib/wiki-types"
import { useWikiData, useWikiQuery } from "./wiki-data"
import { WikiDate, WikiNoteReader } from "./wiki-note-reader"
import { WikiEmptyState } from "./wiki-shared"

function JobResultLabel({ job }: { job: WikiJob }) {
  const t = useTranslations("Wiki.v2")
  const old = useTranslations("Wiki.jobs")
  if (job.status === "succeeded" && job.result)
    return (
      <>
        {job.result.outcome === "generated"
          ? t("generated", { count: job.result.outputs.length })
          : t(`outcomes.${job.result.outcome}`)}
      </>
    )
  return (
    <>
      {old(
        `status.${["queued", "running", "failed", "cancelled", "succeeded"].includes(job.status) ? job.status : "unknown"}` as Parameters<
          typeof old
        >[0]
      )}
    </>
  )
}
export function WikiCompileButton() {
  const t = useTranslations("Wiki.v2")
  const { overview, navigate, invalidate } = useWikiData()
  const settings = useWikiQuery<WikiSettingsView>("get_wiki_settings")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [message, setMessage] = useState<string | null>(null)
  const run = async () => {
    setError(null)
    setMessage(null)
    if (overview && overview.pending_memory_count === 0) {
      setMessage(t("outcomes.no_new_input"))
      return
    }
    setBusy(true)
    try {
      const job = await wikiCompileNow(crypto.randomUUID())
      invalidate()
      navigate({ view: "jobs", job: job.id, path: null, source: null })
    } catch (err) {
      setError(toErrorMessage(err))
    } finally {
      setBusy(false)
    }
  }
  const disabled = busy || !settings.data?.enabled
  return (
    <div className="space-y-2">
      <Button size="sm" disabled={disabled} onClick={() => void run()}>
        {busy ? t("starting") : t("organize")}
      </Button>
      {error && (
        <p role="alert" className="max-w-lg text-sm text-destructive">
          {error}
        </p>
      )}
      {message && (
        <p role="status" className="text-sm text-muted-foreground">
          {message}
        </p>
      )}
      {settings.error && (
        <p role="alert" className="text-sm text-destructive">
          {settings.error}
        </p>
      )}
    </div>
  )
}
export function WikiJobsView() {
  const t = useTranslations("Wiki.v2")
  const old = useTranslations("Wiki.jobs")
  const {
    route,
    navigate,
    invalidate,
    jobFilter: status,
    setJobFilter: setStatus,
  } = useWikiData()
  const [limit, setLimit] = useState(50)
  const [actionError, setActionError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const { data, error, loading } = useWikiQuery<WikiListPage<WikiJob>>(
    "wiki_list_jobs_page",
    { status: status === "all" ? undefined : status, offset: 0, limit }
  )
  const detail = useWikiQuery<WikiJob>(
    "wiki_get_job",
    { id: route.job },
    !!route.job
  )
  const job = detail.data
  const mutation = async (kind: "retry" | "cancel") => {
    if (!job) return
    setBusy(true)
    setActionError(null)
    try {
      const updated = await (kind === "retry"
        ? wikiRetryJob(job.id)
        : wikiCancelJob(job.id))
      invalidate()
      navigate({ job: updated.id })
    } catch (err) {
      setActionError(toErrorMessage(err))
    } finally {
      setBusy(false)
    }
  }
  const configError =
    !!job?.error_code &&
    /model|provider|auth|credential|identity/.test(job.error_code)
  if (route.path)
    return (
      <div className="h-full overflow-y-auto">
        <WikiNoteReader key={route.path} path={route.path} />
      </div>
    )
  return (
    <div className="flex h-full min-h-0 min-w-0 flex-col">
      <div className="flex flex-wrap items-start justify-between gap-3 border-b p-3">
        <select
          aria-label={t("jobFilter")}
          className="h-9 rounded-md border bg-background px-3 text-sm"
          value={status}
          onChange={(event) => {
            setStatus(event.target.value)
            setLimit(50)
          }}
        >
          {(
            [
              "all",
              "active",
              "failed",
              "succeeded",
              "generated",
              "no_content",
              "no_new_input",
              "cancelled",
            ] as const
          ).map((value) => (
            <option key={value} value={value}>
              {t(`jobFilters.${value}`)}
            </option>
          ))}
        </select>
        <WikiCompileButton />
      </div>
      <div className="flex min-h-0 flex-1">
        <aside
          className={`min-h-0 overflow-y-auto border-e md:w-80 md:shrink-0 ${route.job ? "hidden md:block" : "w-full"}`}
        >
          {error && (
            <p role="alert" className="p-4 text-sm text-destructive">
              {error}
              <Button variant="outline" onClick={invalidate}>
                {t("retry")}
              </Button>
            </p>
          )}
          {loading && !data && (
            <p role="status" className="p-4">
              {t("loading")}
            </p>
          )}
          {data && !data.items.length && (
            <WikiEmptyState title={t("noJobs")} description={t("noJobsHint")} />
          )}
          <ul className="space-y-2 p-2">
            {data?.items.map((item) => (
              <li key={item.id}>
                <button
                  type="button"
                  aria-current={route.job === item.id ? "page" : undefined}
                  className={`w-full space-y-2 rounded-lg p-3 text-start hover:bg-muted/60 ${route.job === item.id ? "bg-muted" : ""}`}
                  onClick={() => {
                    setActionError(null)
                    navigate({ job: item.id })
                  }}
                >
                  <span className="block text-sm font-semibold">
                    {old(`kind.${wikiJobKindKey(item.kind)}`)}
                    {item.title ? ` · ${item.title}` : ""}
                  </span>
                  <Badge
                    variant={
                      item.status === "failed" ? "destructive" : "secondary"
                    }
                  >
                    <JobResultLabel job={item} />
                  </Badge>
                  <span className="block text-xs text-muted-foreground">
                    <WikiDate value={item.updated_at ?? item.created_at} />
                  </span>
                </button>
              </li>
            ))}
          </ul>
          {(data?.items.length ?? 0) < (data?.total ?? 0) && (
            <Button
              variant="outline"
              className="m-3"
              disabled={loading}
              onClick={() => setLimit((value) => value + 50)}
            >
              {t("loadMore")}
            </Button>
          )}
        </aside>
        <div
          className={`min-h-0 min-w-0 flex-1 overflow-y-auto p-4 md:p-6 ${route.job ? "" : "hidden md:block"}`}
        >
          {route.job && (
            <Button
              size="sm"
              variant="ghost"
              className="mb-4"
              onClick={() => navigate({ job: null })}
            >
              <ArrowLeft className="size-4 rtl:rotate-180" />
              {t("backToList")}
            </Button>
          )}
          {!route.job && (
            <WikiEmptyState
              title={t("selectJob")}
              description={t("noJobsHint")}
            />
          )}
          {detail.error && (
            <p role="alert" className="text-sm text-destructive">
              {detail.error}
            </p>
          )}
          {detail.loading && !job && <p role="status">{t("loading")}</p>}
          {job && (
            <div className="mx-auto max-w-3xl space-y-5">
              <h1 className="text-xl font-semibold">
                {job.title || old(`kind.${wikiJobKindKey(job.kind)}`)}
              </h1>
              <Badge
                variant={job.status === "failed" ? "destructive" : "secondary"}
              >
                <JobResultLabel job={job} />
              </Badge>
              <dl className="grid grid-cols-[auto_1fr] gap-x-5 gap-y-2 text-sm">
                <dt className="text-muted-foreground">{old("startedAt")}</dt>
                <dd>
                  <WikiDate value={job.started_at ?? job.created_at} />
                </dd>
                <dt className="text-muted-foreground">{old("finishedAt")}</dt>
                <dd>
                  <WikiDate value={job.finished_at} />
                </dd>
                {job.next_attempt_at && (
                  <>
                    <dt>{t("nextAttempt")}</dt>
                    <dd>
                      <WikiDate value={job.next_attempt_at} />
                    </dd>
                  </>
                )}
              </dl>
              {job.status === "failed" && (
                <div
                  role="alert"
                  className="space-y-2 rounded-lg border border-destructive/30 p-4 text-sm"
                >
                  <p className="font-medium">
                    {configError
                      ? t("modelUnavailable")
                      : job.error_code?.includes("read")
                        ? t("inputReadFailed")
                        : t("processingFailed")}
                  </p>
                  {wikiJobErrorMessage(job) && (
                    <p className="break-words">{wikiJobErrorMessage(job)}</p>
                  )}
                </div>
              )}
              {job.result && (
                <>
                  <h2 className="font-semibold">{t("products")}</h2>
                  {job.result.outputs.length ? (
                    <ul className="space-y-2">
                      {job.result.outputs.map((output) => (
                        <li key={`${output.note_id}:${output.path}`}>
                          <button
                            type="button"
                            disabled={
                              !!(
                                job.output_availability?.[output.path] ??
                                output.availability
                              ) &&
                              (job.output_availability?.[output.path] ??
                                output.availability) !== "available"
                            }
                            className="text-start text-sm text-primary underline disabled:text-muted-foreground disabled:no-underline"
                            onClick={() => navigate({ path: output.path })}
                          >
                            {output.title || t("untitled")}
                          </button>
                          {(job.output_availability?.[output.path] ??
                            output.availability) &&
                            (job.output_availability?.[output.path] ??
                              output.availability) !== "available" && (
                              <span className="ms-2 text-xs text-muted-foreground">
                                {t(
                                  (job.output_availability?.[output.path] ??
                                    output.availability) === "missing"
                                    ? "availability.missing"
                                    : "availability.conflict"
                                )}
                              </span>
                            )}
                        </li>
                      ))}
                    </ul>
                  ) : (
                    <p className="text-sm text-muted-foreground">
                      {t(
                        `outcomes.${job.result.outcome === "generated" ? "no_content" : job.result.outcome}`
                      )}
                    </p>
                  )}
                  {job.result.reason_code && (
                    <p className="text-sm text-muted-foreground">
                      {t.has(
                        `reasons.${job.result.reason_code}` as Parameters<
                          typeof t
                        >[0]
                      )
                        ? t(
                            `reasons.${job.result.reason_code}` as Parameters<
                              typeof t
                            >[0]
                          )
                        : t("noContentReason")}
                    </p>
                  )}
                  {job.result.remaining_inputs.length > 0 && (
                    <p className="text-sm">
                      {t("remainingInputs", {
                        count: job.result.remaining_inputs.length,
                      })}
                    </p>
                  )}
                  {job.result.warnings.length > 0 && (
                    <ul className="list-disc space-y-1 ps-5 text-sm text-muted-foreground">
                      {job.result.warnings.map((warning, index) => (
                        <li key={index}>{warning}</li>
                      ))}
                    </ul>
                  )}
                </>
              )}
              <div className="flex flex-wrap gap-2">
                {configError ? (
                  <Button asChild size="sm">
                    <Link href="/settings/wiki?from=wiki">
                      {t("checkModel")}
                    </Link>
                  </Button>
                ) : job.status === "failed" || job.status === "cancelled" ? (
                  <Button
                    size="sm"
                    disabled={busy}
                    onClick={() => void mutation("retry")}
                  >
                    {t("retry")}
                  </Button>
                ) : null}
                {["queued", "running"].includes(job.status) && (
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={busy}
                    onClick={() => void mutation("cancel")}
                  >
                    {t("cancel")}
                  </Button>
                )}
                {job.source_id && (
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() =>
                      navigate({
                        view: "sources",
                        source: job.source_id!,
                        path: null,
                        job: null,
                      })
                    }
                  >
                    {t("viewSources")}
                  </Button>
                )}
              </div>
              {actionError && (
                <p role="alert" className="text-sm text-destructive">
                  {actionError}
                </p>
              )}
              {(job.attempts?.length ?? 0) > 1 && (
                <details className="rounded-lg border p-3 text-sm">
                  <summary className="cursor-pointer">
                    {old("attempt", {
                      n: job.attempt ?? job.attempts?.length ?? 1,
                    })}
                  </summary>
                  <ol className="mt-3 space-y-3">
                    {job.attempts?.map((attempt) => (
                      <li key={attempt.attempt}>
                        <p className="font-medium">
                          {old("attempt", { n: attempt.attempt })}
                        </p>
                        <WikiDate value={attempt.started_at} />
                        {attempt.error_message && (
                          <p className="mt-1 break-words text-muted-foreground">
                            {attempt.error_message}
                          </p>
                        )}
                      </li>
                    ))}
                  </ol>
                </details>
              )}
              <details className="rounded border p-3 text-xs text-muted-foreground">
                <summary className="cursor-pointer">
                  {t("technicalDetails")}
                </summary>
                <pre className="mt-3 overflow-auto whitespace-pre-wrap break-all">
                  {JSON.stringify(
                    {
                      id: job.id,
                      kind: job.kind,
                      attempt: job.attempt,
                      error_code: job.error_code,
                      input_manifest: job.input_manifest,
                      output_manifest: job.output_manifest,
                    },
                    null,
                    2
                  )}
                </pre>
              </details>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
