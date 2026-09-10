"use client"

import { useCallback, useState } from "react"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { formatJson, type JsonFormatMode } from "./json-format.core"
import { ToolCheckbox, ToolParams, ToolSelect } from "./tool-controls"

const EXAMPLE = `{
  "hello": "world",
  "id": 9007199254740993
}`

export default function JsonFormatTool() {
  const [input, setInput] = useState("")
  const [mode, setMode] = useState<JsonFormatMode>("pretty")
  const [stringifyBigIntegers, setStringifyBigIntegers] = useState(false)
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const computed = formatJson(input, { mode, stringifyBigIntegers })

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      example={EXAMPLE}
      result={computed.ok ? computed.output : ""}
      error={computed.ok ? null : computed.error}
      downloadFilename="formatted.json"
      params={
        <>
          <ToolParams>
            <ToolSelect
              label="Mode"
              value={mode}
              onChange={(value) => setMode(value as JsonFormatMode)}
              options={[
                { value: "pretty", label: "Pretty" },
                { value: "minify", label: "Minify" },
              ]}
            />
            <ToolCheckbox
              label="Stringify big integers"
              checked={stringifyBigIntegers}
              onChange={setStringifyBigIntegers}
            />
          </ToolParams>
          {computed.ok && computed.warning ? (
            <p className="text-xs text-amber-700 dark:text-amber-500">
              {computed.warning}
            </p>
          ) : null}
        </>
      }
    />
  )
}
