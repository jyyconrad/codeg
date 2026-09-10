"use client"

import { useCallback, useState } from "react"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { testRegex } from "./regex-tester.core"
import { ToolParams, ToolText } from "./tool-controls"

export default function RegexTesterTool() {
  const [input, setInput] = useState("")
  const [pattern, setPattern] = useState("")
  const [flags, setFlags] = useState("g")
  const [replacement, setReplacement] = useState("")
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const computed = testRegex(input, { pattern, flags, replacement })

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      inputLabel="Test string"
      onExample={() => {
        setPattern("(\\w+)@(\\w+\\.\\w+)")
        setFlags("g")
        setReplacement("$1 at $2")
        setInput("a@b.com and c@d.org")
      }}
      result={computed.ok ? computed.output : ""}
      error={computed.ok ? null : computed.error}
      downloadFilename="regex-result.txt"
      params={
        <ToolParams>
          <ToolText
            label="Pattern"
            value={pattern}
            onChange={setPattern}
            placeholder="(foo)+"
            mono
          />
          <ToolText
            label="Flags"
            value={flags}
            onChange={setFlags}
            placeholder="gimsuy"
            mono
          />
          <ToolText
            label="Replace"
            value={replacement}
            onChange={setReplacement}
            placeholder="$1"
            mono
          />
        </ToolParams>
      }
    />
  )
}
