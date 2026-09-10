"use client"

import { useState, type ReactNode } from "react"
import { useTranslations } from "next-intl"
import { Button } from "@/components/ui/button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Textarea } from "@/components/ui/textarea"
import { useCopiedFlag } from "@/hooks/use-copied-flag"
import { cn } from "@/lib/utils"
import { getToolboxTool } from "./registry"
import { useToolboxStore } from "./toolbox-store"
import type { ToolboxToolId } from "./types"

export const TOOL_IO_FRAME_CLASS =
  "flex h-[300px] max-h-[calc(100dvh-6rem)] min-h-0 w-full flex-col overflow-auto"

export const TOOL_IO_TEXTAREA_CLASS =
  "h-full min-h-0 flex-1 field-sizing-fixed resize-none overflow-y-auto font-mono text-sm"

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
  const [sendToKey, setSendToKey] = useState(0)
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
    <div
      className={cn(
        "flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-4",
        className
      )}
    >
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
          <div className="ml-auto flex items-center gap-2 text-xs text-muted-foreground">
            {t("sendTo")}
            <Select
              key={sendToKey}
              onValueChange={(next) => {
                sendResultTo(next as ToolboxToolId, result)
                setSendToKey((current) => current + 1)
              }}
            >
              <SelectTrigger size="sm" aria-label={t("sendTo")}>
                <SelectValue placeholder={t("sendToPlaceholder")} />
              </SelectTrigger>
              <SelectContent position="popper" align="end">
                {meta.chainTargets.map((id) => (
                  <SelectItem key={id} value={id}>
                    {t(`tools.${id}.title`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        ) : null}
      </div>
      {params}
      <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
        <div className="flex min-w-0 flex-col gap-1.5">
          <span className="text-xs font-medium text-muted-foreground">
            {inputLabel ?? t("input")}
          </span>
          <div className={TOOL_IO_FRAME_CLASS}>
            {inputSlot ?? (
              <Textarea
                value={input}
                onChange={(event) => onInputChange(event.target.value)}
                className={TOOL_IO_TEXTAREA_CLASS}
              />
            )}
          </div>
        </div>
        <div className="flex min-w-0 flex-col gap-1.5">
          <span className="text-xs font-medium text-muted-foreground">
            {t("output")}
          </span>
          <div className={TOOL_IO_FRAME_CLASS}>
            {resultSlot ?? (
              <Textarea
                readOnly
                value={error ? "" : result}
                className={TOOL_IO_TEXTAREA_CLASS}
              />
            )}
          </div>
        </div>
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
