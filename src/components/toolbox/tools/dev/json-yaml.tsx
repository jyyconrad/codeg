"use client"

import { useCallback, useState } from "react"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { convertJsonYaml, type JsonYamlDirection } from "./json-yaml.core"
import { ToolParams, ToolSelect } from "./tool-controls"

const JSON_EXAMPLE = `{
  "name": "Ada",
  "ok": true,
  "items": [1, "two"]
}`

const YAML_EXAMPLE = `name: Ada
ok: true
items:
  - 1
  - two
`

export default function JsonYamlTool() {
  const [input, setInput] = useState("")
  const [direction, setDirection] = useState<JsonYamlDirection>("json-to-yaml")
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const computed = convertJsonYaml(input, direction)

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      example={direction === "json-to-yaml" ? JSON_EXAMPLE : YAML_EXAMPLE}
      result={computed.ok ? computed.output : ""}
      error={computed.ok ? null : computed.error}
      downloadFilename={
        direction === "json-to-yaml" ? "converted.yaml" : "converted.json"
      }
      params={
        <ToolParams>
          <ToolSelect
            label="Direction"
            value={direction}
            onChange={(value) => setDirection(value as JsonYamlDirection)}
            options={[
              { value: "json-to-yaml", label: "JSON to YAML" },
              { value: "yaml-to-json", label: "YAML to JSON" },
            ]}
          />
        </ToolParams>
      }
    />
  )
}
