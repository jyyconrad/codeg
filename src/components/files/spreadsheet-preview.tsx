"use client"

import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslations } from "next-intl"
import { ChevronLeft, ChevronRight, FileWarning, Loader2 } from "lucide-react"

import { usePreviewFileChanges } from "@/hooks/use-preview-file-changes"
import { readSpreadsheetPreview } from "@/lib/api"
import { extractAppCommandError } from "@/lib/app-error"
import {
  clampColLimit,
  clampOffset,
  clampRowLimit,
  DEFAULT_COL_LIMIT,
  DEFAULT_ROW_LIMIT,
  nextColOffset,
  nextRowOffset,
  pageRowRange,
  prevColOffset,
  prevRowOffset,
} from "@/lib/spreadsheet-preview"
import type { SpreadsheetPreviewPage } from "@/lib/types"
import { cn } from "@/lib/utils"

const PAGE_SIZES = [50, 100, 200] as const

export function SpreadsheetPreview({
  path,
  rootPath,
  relPath,
}: {
  path: string
  rootPath: string | null
  relPath: string | null
}) {
  const t = useTranslations("Folder.fileWorkspacePanel")
  const [sheet, setSheet] = useState<string | null>(null)
  const [rowOffset, setRowOffset] = useState(0)
  const [colOffset, setColOffset] = useState(0)
  const [rowLimit, setRowLimit] = useState(DEFAULT_ROW_LIMIT)
  const [reloadKey, setReloadKey] = useState(0)
  const [page, setPage] = useState<SpreadsheetPreviewPage | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    setSheet(null)
    setRowOffset(0)
    setColOffset(0)
  }, [path])

  const load = useCallback(async () => {
    if (!rootPath || !relPath) {
      setError(t("spreadsheetLoadFailed"))
      setLoading(false)
      return
    }
    setLoading(true)
    try {
      const next = await readSpreadsheetPreview({
        rootPath,
        path: relPath,
        sheet,
        rowOffset: clampOffset(rowOffset),
        rowLimit: clampRowLimit(rowLimit),
        colOffset: clampOffset(colOffset),
        colLimit: clampColLimit(DEFAULT_COL_LIMIT),
      })
      setPage(next)
      setError(null)
    } catch (err) {
      setError(
        extractAppCommandError(err)?.message ?? t("spreadsheetLoadFailed")
      )
    } finally {
      setLoading(false)
    }
    void reloadKey
  }, [rootPath, relPath, sheet, rowOffset, rowLimit, colOffset, t, reloadKey])

  useEffect(() => {
    void load()
  }, [load])

  usePreviewFileChanges(path, () => setReloadKey((k) => k + 1))

  const range = page
    ? pageRowRange(page.rowOffset, page.rows.length, page.totalRows)
    : null
  const canPrev = (page?.rowOffset ?? 0) > 0
  const canNext = page != null && !page.eof
  const canPrevCols = (page?.colOffset ?? 0) > 0
  const canNextCols =
    page != null && page.colOffset + page.colLimit < page.totalColumns
  const showSheetSwitch = (page?.sheets.length ?? 0) > 1

  const colLabels = useMemo(() => {
    if (!page) return []
    const start = page.colOffset
    return page.header.map((_, i) => columnLetter(start + i))
  }, [page])

  if (error && !page) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
        <FileWarning className="h-8 w-8 text-muted-foreground" />
        <div className="text-sm font-medium text-foreground">
          {t("spreadsheetLoadFailed")}
        </div>
        <div className="max-w-sm break-words text-xs text-muted-foreground">
          {error}
        </div>
        <button
          type="button"
          onClick={() => setReloadKey((k) => k + 1)}
          className="mt-1 rounded-md border border-border bg-card px-3 py-1.5 text-xs font-medium text-foreground transition-colors hover:bg-primary/8"
        >
          {t("spreadsheetRetry")}
        </button>
      </div>
    )
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border/60 px-2 py-1.5 text-xs">
        {showSheetSwitch && page && (
          <label className="flex items-center gap-1.5">
            <span className="text-muted-foreground">{t("spreadsheetSheet")}</span>
            <select
              className="rounded-md border border-border bg-background px-1.5 py-0.5"
              value={page.sheet}
              onChange={(e) => {
                setSheet(e.target.value)
                setRowOffset(0)
                setColOffset(0)
              }}
            >
              {page.sheets.map((item) => (
                <option key={item.name} value={item.name}>
                  {item.name}
                </option>
              ))}
            </select>
          </label>
        )}
        <select
          className="rounded-md border border-border bg-background px-1.5 py-0.5"
          value={rowLimit}
          aria-label={t("spreadsheetPageSize")}
          onChange={(e) => {
            setRowLimit(clampRowLimit(Number(e.target.value)))
            setRowOffset(0)
          }}
        >
          {PAGE_SIZES.map((size) => (
            <option key={size} value={size}>
              {size}
            </option>
          ))}
        </select>
        {range && page && (
          <span className="text-muted-foreground">
            {t("spreadsheetRows", {
              from: range.from,
              to: range.to,
              total: page.totalRows,
            })}
          </span>
        )}
        <div className="ml-auto flex items-center gap-1">
          {page && page.totalColumns > DEFAULT_COL_LIMIT && (
            <>
              <ToolbarButton
                disabled={!canPrevCols}
                onClick={() =>
                  setColOffset(
                    prevColOffset(page.colOffset, DEFAULT_COL_LIMIT)
                  )
                }
                label={t("spreadsheetPrevCols")}
              >
                <ChevronLeft className="h-3.5 w-3.5" />
              </ToolbarButton>
              <ToolbarButton
                disabled={!canNextCols}
                onClick={() =>
                  setColOffset(
                    nextColOffset(
                      page.colOffset,
                      DEFAULT_COL_LIMIT,
                      page.totalColumns
                    )
                  )
                }
                label={t("spreadsheetNextCols")}
              >
                <ChevronRight className="h-3.5 w-3.5" />
              </ToolbarButton>
            </>
          )}
          <ToolbarButton
            disabled={!canPrev}
            onClick={() =>
              page && setRowOffset(prevRowOffset(page.rowOffset, rowLimit))
            }
            label={t("spreadsheetPrevPage")}
          >
            <ChevronLeft className="h-3.5 w-3.5" />
          </ToolbarButton>
          <ToolbarButton
            disabled={!canNext}
            onClick={() =>
              page &&
              setRowOffset(
                nextRowOffset(page.rowOffset, rowLimit, page.totalRows)
              )
            }
            label={t("spreadsheetNextPage")}
          >
            <ChevronRight className="h-3.5 w-3.5" />
          </ToolbarButton>
        </div>
      </div>
      <div className="relative min-h-0 flex-1 overflow-auto">
        {loading && (
          <div className="absolute right-2 top-2 z-10 flex items-center gap-1 rounded-md bg-background/70 px-2 py-1 text-2xs text-muted-foreground backdrop-blur-sm">
            <Loader2 className="h-3 w-3 animate-spin" />
            {t("loading")}
          </div>
        )}
        {page && page.totalRows === 0 ? (
          <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
            {t("spreadsheetEmpty")}
          </div>
        ) : page ? (
          <table className="min-w-full border-collapse text-xs">
            <thead className="sticky top-0 z-[1] bg-muted">
              <tr>
                <th className="sticky left-0 z-[2] border-b border-r border-border bg-muted px-2 py-1 text-left font-medium text-muted-foreground">
                  #
                </th>
                {page.header.map((cell, i) => (
                  <th
                    key={`${colLabels[i]}-h`}
                    className="border-b border-border px-2 py-1 text-left font-medium"
                  >
                    <div className="text-2xs text-muted-foreground">
                      {colLabels[i]}
                    </div>
                    {cell}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {page.rows.map((row, r) => {
                const sheetRow = page.rowOffset + r + 2
                return (
                  <tr key={sheetRow} className="odd:bg-background even:bg-muted/30">
                    <th className="sticky left-0 border-r border-border bg-inherit px-2 py-1 text-left font-normal text-muted-foreground">
                      {sheetRow}
                    </th>
                    {row.map((cell, c) => (
                      <td
                        key={`${sheetRow}-${c}`}
                        className="max-w-[20rem] truncate px-2 py-1"
                      >
                        {cell}
                      </td>
                    ))}
                  </tr>
                )
              })}
            </tbody>
          </table>
        ) : (
          <div className="flex h-full items-center justify-center gap-2 text-xs text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            {t("loading")}
          </div>
        )}
      </div>
    </div>
  )
}

function ToolbarButton({
  disabled,
  onClick,
  label,
  children,
}: {
  disabled: boolean
  onClick: () => void
  label: string
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      aria-label={label}
      title={label}
      className={cn(
        "flex h-6 w-6 items-center justify-center rounded hover:bg-primary/8 disabled:opacity-40"
      )}
    >
      {children}
    </button>
  )
}

function columnLetter(index: number): string {
  let n = index + 1
  let out = ""
  while (n > 0) {
    const rem = (n - 1) % 26
    out = String.fromCharCode(65 + rem) + out
    n = Math.floor((n - 1) / 26)
  }
  return out
}
