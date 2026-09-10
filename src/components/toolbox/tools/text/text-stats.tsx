"use client"

import { useCallback, useMemo, useState } from "react"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import { computeTextStats, formatTextStats } from "./text-stats-core"

const EXAMPLE = "Hello, 世界!\nThis is a test."

export default function TextStatsTool() {
  const [input, setInput] = useState("")
  const [countWhitespace, setCountWhitespace] = useState(true)
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const stats = useMemo(
    () => computeTextStats(input, countWhitespace),
    [input, countWhitespace]
  )
  const result = formatTextStats(stats)

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      example={EXAMPLE}
      result={result}
      downloadFilename="text-stats.txt"
      params={
        <Label className="flex items-center gap-2 text-xs font-normal text-muted-foreground">
          <Checkbox
            checked={countWhitespace}
            onCheckedChange={(value) => setCountWhitespace(value === true)}
          />
          Count whitespace
        </Label>
      }
      resultSlot={
        <dl className="min-h-[12rem] flex-1 space-y-2 rounded-xl border border-border bg-input/30 p-3 font-mono text-sm">
          {(
            [
              ["Characters", stats.characters],
              ["CJK characters", stats.cjk],
              ["Words", stats.words],
              ["Lines", stats.lines],
              ["Bytes (UTF-8)", stats.bytes],
            ] as const
          ).map(([label, value]) => (
            <div
              key={label}
              className="flex items-baseline justify-between gap-4"
            >
              <dt className="text-muted-foreground">{label}</dt>
              <dd>{value}</dd>
            </div>
          ))}
        </dl>
      }
    />
  )
}
