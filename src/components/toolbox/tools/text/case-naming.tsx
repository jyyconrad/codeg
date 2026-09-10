"use client"

import { useCallback, useMemo, useState } from "react"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import {
  convertNaming,
  NAMING_STYLES,
  type NamingStyle,
} from "./case-naming-core"

const SELECT_CLASS =
  "h-8 rounded-full border border-border bg-input/30 px-2 text-xs text-foreground"

const EXAMPLE = "hello_world\nXMLHttpRequest\nfoo-bar-baz"

export default function CaseNamingTool() {
  const [input, setInput] = useState("")
  const [style, setStyle] = useState<NamingStyle>("camelCase")
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const result = useMemo(() => convertNaming(input, style), [input, style])

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      example={EXAMPLE}
      result={result}
      downloadFilename="naming.txt"
      params={
        <label className="flex items-center gap-2 text-xs text-muted-foreground">
          Style
          <select
            className={SELECT_CLASS}
            value={style}
            onChange={(event) => setStyle(event.target.value as NamingStyle)}
          >
            {NAMING_STYLES.map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </select>
        </label>
      }
    />
  )
}
