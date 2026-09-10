"use client"

import { useCallback, useMemo, useState } from "react"
import { useTranslations } from "next-intl"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Input } from "@/components/ui/input"
import { TOOL_LABEL_CLASS } from "@/components/toolbox/tools/image/image-io"
import {
  diffDates,
  formatDateDiff,
  parseDateDiffInput,
  parseToolDate,
  toDateTimeLocalValue,
} from "./date-diff"

function defaultRange(): { start: string; end: string } {
  const end = new Date()
  const start = new Date(end.getTime() - 7 * 24 * 60 * 60 * 1000)
  return {
    start: toDateTimeLocalValue(start),
    end: toDateTimeLocalValue(end),
  }
}

export default function DateDiffTool() {
  const t = useTranslations("Toolbox")
  const initial = defaultRange()
  const [input, setInput] = useState(`${initial.start}\n${initial.end}`)
  const [start, setStart] = useState(initial.start)
  const [end, setEnd] = useState(initial.end)
  const [inclusive, setInclusive] = useState(false)

  const applyRange = useCallback((nextStart: string, nextEnd: string) => {
    setStart(nextStart)
    setEnd(nextEnd)
    setInput(`${nextStart}\n${nextEnd}`)
  }, [])

  const onInput = useCallback((value: string) => {
    setInput(value)
    const parsed = parseDateDiffInput(value)
    if (parsed) {
      setStart(parsed.start)
      setEnd(parsed.end)
    }
    if (!value) {
      setStart("")
      setEnd("")
    }
  }, [])
  useToolPendingInput(onInput)

  const { result, error } = useMemo(() => {
    if (!start || !end) return { result: "", error: null }
    const startDate = parseToolDate(start)
    const endDate = parseToolDate(end)
    if (!startDate || !endDate) {
      return { result: "", error: "Invalid date." }
    }
    return {
      result: formatDateDiff(diffDates(startDate, endDate, inclusive)),
      error: null,
    }
  }, [start, end, inclusive])

  return (
    <ToolPageShell
      input={input}
      onInputChange={onInput}
      inputLabel={t("input")}
      result={result}
      error={error}
      onExample={() => {
        const range = defaultRange()
        applyRange(range.start, range.end)
      }}
      params={
        <div className="flex flex-col gap-2">
          <p className="text-xs font-medium text-muted-foreground">
            {t("params")}
          </p>
          <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <input
              type="checkbox"
              checked={inclusive}
              onChange={(event) => setInclusive(event.target.checked)}
            />
            Inclusive
          </label>
        </div>
      }
      inputSlot={
        <div className="flex min-h-[12rem] flex-1 flex-col gap-2">
          <label className={TOOL_LABEL_CLASS}>
            Start
            <Input
              type="datetime-local"
              value={start}
              onChange={(event) => applyRange(event.target.value, end)}
            />
          </label>
          <label className={TOOL_LABEL_CLASS}>
            End
            <Input
              type="datetime-local"
              value={end}
              onChange={(event) => applyRange(start, event.target.value)}
            />
          </label>
        </div>
      }
    />
  )
}
