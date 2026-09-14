"use client"

import { useCallback, useMemo, useState } from "react"
import { ToolboxSelect } from "@/components/toolbox/toolbox-controls"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import {
  convertNaming,
  NAMING_STYLES,
  type NamingStyle,
} from "./case-naming-core"

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
        <ToolboxSelect
          label="Style"
          value={style}
          onChange={(value) => setStyle(value as NamingStyle)}
          options={NAMING_STYLES.map((name) => ({
            value: name,
            label: name,
          }))}
        />
      }
    />
  )
}
