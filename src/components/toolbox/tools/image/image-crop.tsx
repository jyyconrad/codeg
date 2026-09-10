"use client"

import { useCallback, useState } from "react"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"

export default function ImageCropTool() {
  const [input, setInput] = useState("")
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      result={input}
    />
  )
}
