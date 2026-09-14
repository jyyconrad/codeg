"use client"

import { useCallback, useState } from "react"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { formatCode, type CodeFormatKind } from "./code-format.core"
import { ToolParams, ToolSelect } from "./tool-controls"

const XML_EXAMPLE = `<?xml version="1.0"?>
<root>
  <item id="1">hello</item>
</root>`

const YAML_EXAMPLE = `foo: bar
list:
  - 1
  - two
`

export default function CodeFormatTool() {
  const [input, setInput] = useState("")
  const [kind, setKind] = useState<CodeFormatKind>("xml")
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const computed = formatCode(input, kind)

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      example={kind === "xml" ? XML_EXAMPLE : YAML_EXAMPLE}
      result={computed.ok ? computed.output : ""}
      error={computed.ok ? null : computed.error}
      downloadFilename={kind === "xml" ? "formatted.xml" : "formatted.yaml"}
      params={
        <ToolParams>
          <ToolSelect
            label="Format"
            value={kind}
            onChange={(value) => setKind(value as CodeFormatKind)}
            options={[
              { value: "xml", label: "XML" },
              { value: "yaml", label: "YAML" },
            ]}
          />
        </ToolParams>
      }
    />
  )
}
