"use client"

import { useCallback, useMemo, useState } from "react"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { convertZh, type ZhDirection } from "./zh-convert-core"

const SELECT_CLASS =
  "h-8 rounded-full border border-border bg-input/30 px-2 text-xs text-foreground"

const EXAMPLE = "汉字转换：后面是头发。"

export default function ZhConvertTool() {
  const [input, setInput] = useState("")
  const [direction, setDirection] = useState<ZhDirection>("s2t")
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const result = useMemo(() => convertZh(input, direction), [input, direction])

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      example={EXAMPLE}
      result={result}
      downloadFilename="zh-converted.txt"
      params={
        <label className="flex items-center gap-2 text-xs text-muted-foreground">
          Direction
          <select
            className={SELECT_CLASS}
            value={direction}
            onChange={(event) =>
              setDirection(event.target.value as ZhDirection)
            }
          >
            <option value="s2t">Simplified → Traditional</option>
            <option value="t2s">Traditional → Simplified</option>
          </select>
        </label>
      }
    />
  )
}
