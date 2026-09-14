"use client"

import { useCallback, useMemo, useState } from "react"
import { ToolboxSelect } from "@/components/toolbox/toolbox-controls"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { convertZh, type ZhDirection } from "./zh-convert-core"

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
        <ToolboxSelect
          label="Direction"
          value={direction}
          onChange={(value) => setDirection(value as ZhDirection)}
          options={[
            { value: "s2t", label: "Simplified → Traditional" },
            { value: "t2s", label: "Traditional → Simplified" },
          ]}
        />
      }
    />
  )
}
