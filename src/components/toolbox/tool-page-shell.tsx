"use client"

import { type ReactNode } from "react"
import { useTranslations } from "next-intl"
import { Button } from "@/components/ui/button"
import { Textarea } from "@/components/ui/textarea"
import { useCopiedFlag } from "@/hooks/use-copied-flag"
import { cn } from "@/lib/utils"
import { getToolboxTool } from "./registry"
import { useToolboxStore } from "./toolbox-store"
import type { ToolboxToolId } from "./types"

export function ToolPageShell({
  input,
  onInputChange,
  inputLabel,
  inputSlot,
  params,
  result,
  resultSlot,
  error,
  example,
  onExample,
  encodingNotice,
  cryptoFooter,
  downloadFilename = "toolbox-result.txt",
  className,
}: {
  input: string
  onInputChange: (value: string) => void
  inputLabel?: string
  inputSlot?: ReactNode
  params?: ReactNode
  result: string
  resultSlot?: ReactNode
  error?: string | null
  example?: string
  onExample?: () => void
  encodingNotice?: boolean
  cryptoFooter?: boolean
  downloadFilename?: string
  className?: string
}) {
  const t = useTranslations("Toolbox")
  const [copied, markCopied] = useCopiedFlag()
  const sendResultTo = useToolboxStore((s) => s.sendResultTo)
  const selectedToolId = useToolboxStore((s) => s.selectedToolId)
  const meta = selectedToolId ? getToolboxTool(selectedToolId) : undefined

  async function copyResult() {
    if (!result) return
    try {
      await navigator.clipboard.writeText(result)
      markCopied()
    } catch {
      /* clipboard may be denied */
    }
  }

  function downloadResult() {
    if (!result) return
    const blob = new Blob([result], { type: "text/plain;charset=utf-8" })
    const url = URL.createObjectURL(blob)
    const link = document.createElement("a")
    link.href = url
    link.download = downloadFilename
    link.click()
    URL.revokeObjectURL(url)
  }

  function clearAll() {
    onInputChange("")
  }

  return (
    <div className={cn("flex min-h-0 flex-1 flex-col gap-3 p-4", className)}>
      {encodingNotice ? (
        <p className="text-xs text-muted-foreground">{t("encodingNotice")}</p>
      ) : null}
      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => {
            if (onExample) onExample()
            else if (example != null) onInputChange(example)
          }}
          disabled={!onExample && example == null}
        >
          {t("example")}
        </Button>
        <Button type="button" variant="outline" size="sm" onClick={clearAll}>
          {t("clear")}
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() => void copyResult()}
          disabled={!result}
        >
          {copied ? t("copied") : t("copy")}
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={downloadResult}
          disabled={!result}
        >
          {t("download")}
        </Button>
        {meta && meta.chainTargets.length > 0 && result ? (
          <label className="ml-auto flex items-center gap-2 text-xs text-muted-foreground">
            {t("sendTo")}
            <select
              className="h-8 rounded-full border border-border bg-input/30 px-2 text-foreground"
              defaultValue=""
              onChange={(event) => {
                const next = event.target.value as ToolboxToolId
                if (!next) return
                sendResultTo(next, result)
                event.target.value = ""
              }}
            >
              <option value="">{t("sendToPlaceholder")}</option>
              {meta.chainTargets.map((id) => (
                <option key={id} value={id}>
                  {t(`tools.${id}.title`)}
                </option>
              ))}
            </select>
          </label>
        ) : null}
      </div>
      {params}
      <div className="grid min-h-0 flex-1 grid-cols-1 gap-3 lg:grid-cols-2">
        <label className="flex min-h-0 flex-col gap-1.5">
          <span className="text-xs font-medium text-muted-foreground">
            {inputLabel ?? t("input")}
          </span>
          {inputSlot ?? (
            <Textarea
              value={input}
              onChange={(event) => onInputChange(event.target.value)}
              className="min-h-[12rem] flex-1 font-mono text-sm"
            />
          )}
        </label>
        <label className="flex min-h-0 flex-col gap-1.5">
          <span className="text-xs font-medium text-muted-foreground">
            {t("output")}
          </span>
          {resultSlot ?? (
            <Textarea
              readOnly
              value={error ? "" : result}
              className="min-h-[12rem] flex-1 font-mono text-sm"
            />
          )}
        </label>
      </div>
      {error ? (
        <p className="text-sm text-destructive" role="alert">
          {t("error")}: {error}
        </p>
      ) : null}
      {cryptoFooter ? (
        <p className="text-xs text-muted-foreground">{t("cryptoFooter")}</p>
      ) : null}
    </div>
  )
}
