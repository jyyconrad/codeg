/**
 * Wiki 前端共享状态：同步查询参数、浏览器历史、后台事件和请求刷新代次。
 * 各视图经 Transport 读取后端事实；统一丢弃过期响应，并允许目录生成完成后再刷新正文。
 */
"use client"

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useState,
  type ReactNode,
} from "react"
import { getTransport } from "@/lib/transport"
import { toErrorMessage } from "@/lib/app-error"
import type { WikiOverview } from "@/lib/wiki-types"

export type WikiViewId =
  | "library"
  | "overview"
  | "work"
  | "capabilities"
  | "sources"
  | "jobs"
export interface WikiRoute {
  view: WikiViewId
  path: string | null
  source: string | null
  job: string | null
  query: string
  scope: "notes" | "sources" | "all"
}
const initialRoute: WikiRoute = {
  view: "library",
  path: null,
  source: null,
  job: null,
  query: "",
  scope: "notes",
}
function readRoute(): WikiRoute {
  if (typeof window === "undefined") return initialRoute
  const params = new URLSearchParams(window.location.search)
  const view = params.get("wikiView")
  const scope = params.get("wikiScope")
  return {
    view: [
      "library",
      "overview",
      "work",
      "capabilities",
      "sources",
      "jobs",
    ].includes(view ?? "")
      ? (view as WikiViewId)
      : "library",
    path: params.get("wikiPath"),
    source: params.get("wikiSource"),
    job: params.get("wikiJob"),
    query: params.get("wikiQuery") ?? "",
    scope: scope === "all" || scope === "sources" ? scope : "notes",
  }
}
const WikiDataContext = createContext({
  revision: 0,
  invalidate: () => {},
  route: initialRoute,
  navigate: (() => {}) as (
    patch: Partial<WikiRoute>,
    replace?: boolean
  ) => void,
  jobFilter: "all",
  setJobFilter: (() => {}) as (value: string) => void,
  overview: null as WikiOverview | null,
  overviewError: null as string | null,
})

export function WikiDataProvider({ children }: { children: ReactNode }) {
  const [revision, setRevision] = useState(0)
  const [jobFilter, setJobFilter] = useState("all")
  const [route, setRoute] = useState<WikiRoute>(readRoute)
  const [overview, setOverview] = useState<WikiOverview | null>(null)
  const [overviewError, setOverviewError] = useState<string | null>(null)
  const invalidate = useCallback(() => setRevision((value) => value + 1), [])
  const navigate = useCallback((patch: Partial<WikiRoute>, replace = false) => {
    const previous = readRoute()
    const next = { ...previous, ...patch }
    if (patch.path) {
      if (!patch.view && previous.view !== "library") {
        next.view = patch.path.startsWith("capabilities/")
          ? "capabilities"
          : patch.path.startsWith("work/")
            ? "work"
            : "overview"
      }
      next.source = null
      next.job = null
    } else if (patch.source) {
      next.view = "sources"
      next.path = null
      next.job = null
    } else if (patch.job) {
      next.view = "jobs"
      next.path = null
      next.source = null
    }
    const url = new URL(window.location.href)
    if (next.path !== previous.path || next.source !== previous.source)
      url.hash = ""
    for (const [field, param] of Object.entries({
      view: "wikiView",
      path: "wikiPath",
      source: "wikiSource",
      job: "wikiJob",
      query: "wikiQuery",
      scope: "wikiScope",
    })) {
      const value = next[field as keyof WikiRoute]
      if (value) url.searchParams.set(param, value)
      else url.searchParams.delete(param)
    }
    window.history[replace ? "replaceState" : "pushState"](
      window.history.state,
      "",
      url
    )
    setRoute(next)
  }, [])
  useEffect(() => {
    let stale = false
    getTransport()
      .call<WikiOverview>("wiki_get_overview")
      .then((value) => {
        if (!stale) {
          setOverview(value)
          setOverviewError(null)
        }
      })
      .catch((error) => {
        if (!stale) setOverviewError(toErrorMessage(error))
      })
    return () => {
      stale = true
    }
  }, [revision])
  useEffect(() => {
    let disposed = false
    const cleanups: (() => void)[] = []
    for (const event of ["wiki://job-changed", "wiki://content-changed"]) {
      getTransport()
        .subscribe(event, invalidate)
        .then((unsub) => {
          if (disposed) unsub()
          else cleanups.push(unsub)
        })
        .catch(() => {})
    }
    const reconnect = getTransport().onReconnect?.(invalidate)
    if (reconnect) cleanups.push(reconnect)
    const pop = () => setRoute(readRoute())
    const visible = () => {
      if (document.visibilityState === "visible") invalidate()
    }
    window.addEventListener("popstate", pop)
    window.addEventListener("focus", invalidate)
    window.addEventListener("online", invalidate)
    document.addEventListener("visibilitychange", visible)
    return () => {
      disposed = true
      cleanups.forEach((fn) => fn())
      window.removeEventListener("popstate", pop)
      window.removeEventListener("focus", invalidate)
      window.removeEventListener("online", invalidate)
      document.removeEventListener("visibilitychange", visible)
    }
  }, [invalidate])
  useEffect(() => {
    if (!overview?.active_job_count) return
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") invalidate()
    }, 5000)
    return () => window.clearInterval(timer)
  }, [overview?.active_job_count, invalidate])
  return (
    <WikiDataContext.Provider
      value={{
        revision,
        invalidate,
        route,
        navigate,
        overview,
        overviewError,
        jobFilter,
        setJobFilter,
      }}
    >
      {children}
    </WikiDataContext.Provider>
  )
}
export const useWikiData = () => useContext(WikiDataContext)

/** 默认共享刷新代次；库正文可指定已完成的目录代次，避免读取尚未更新的索引。 */
export function useWikiQuery<T>(
  command: string,
  args: Record<string, unknown> = {},
  enabled = true,
  refreshRevision?: number
) {
  const { revision: sharedRevision, invalidate } = useWikiData()
  const revision = refreshRevision ?? sharedRevision
  const key = JSON.stringify(args)
  const identity = `${command}:${key}`
  const [state, setState] = useState<{
    key: string
    data: T | null
    error: string | null
    generation: number
  }>({ key: identity, data: null, error: null, generation: -1 })
  useEffect(() => {
    if (!enabled) return
    let stale = false
    getTransport()
      .call<T>(command, JSON.parse(key))
      .then((data) => {
        if (!stale)
          setState({ key: identity, data, error: null, generation: revision })
      })
      .catch((error) => {
        if (!stale)
          setState((current) => ({
            key: identity,
            data: current.key === identity ? current.data : null,
            error: toErrorMessage(error),
            generation: revision,
          }))
      })
    return () => {
      stale = true
    }
  }, [command, key, identity, revision, enabled])
  return {
    ...state,
    data: state.key === identity && enabled ? state.data : null,
    error:
      state.key === identity && state.generation === revision
        ? state.error
        : null,
    loading:
      enabled && (state.key !== identity || state.generation !== revision),
    refresh: invalidate,
  }
}
