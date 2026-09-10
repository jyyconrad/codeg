"use client"

import { useCallback, useState } from "react"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import {
  convertJsonCsv,
  peekCsvHeaders,
  type CsvFieldType,
  type JsonCsvDirection,
} from "./json-csv.core"
import { ToolParams, ToolSelect } from "./tool-controls"

const JSON_EXAMPLE = `[
  {"name": "Ada", "age": 36, "ok": true},
  {"name": "Bob", "age": 1, "ok": false}
]`

const CSV_EXAMPLE = `name,age,ok
Ada,36,true
Bob,1,false`

export default function JsonCsvTool() {
  const [input, setInput] = useState("")
  const [direction, setDirection] = useState<JsonCsvDirection>("json-to-csv")
  const [fieldTypes, setFieldTypes] = useState<Record<string, CsvFieldType>>({})
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const headers = direction === "csv-to-json" ? peekCsvHeaders(input) : []
  const computed = convertJsonCsv(input, { direction, fieldTypes })

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      example={direction === "json-to-csv" ? JSON_EXAMPLE : CSV_EXAMPLE}
      result={computed.ok ? computed.output : ""}
      error={computed.ok ? null : computed.error}
      downloadFilename={
        direction === "json-to-csv" ? "converted.csv" : "converted.json"
      }
      params={
        <ToolParams>
          <ToolSelect
            label="Direction"
            value={direction}
            onChange={(value) => setDirection(value as JsonCsvDirection)}
            options={[
              { value: "json-to-csv", label: "JSON to CSV" },
              { value: "csv-to-json", label: "CSV to JSON" },
            ]}
          />
          {headers.map((header) => (
            <ToolSelect
              key={header}
              label={header || "(empty)"}
              value={fieldTypes[header] ?? "string"}
              onChange={(value) =>
                setFieldTypes((current) => ({
                  ...current,
                  [header]: value as CsvFieldType,
                }))
              }
              options={[
                { value: "string", label: "string" },
                { value: "number", label: "number" },
                { value: "boolean", label: "boolean" },
              ]}
            />
          ))}
        </ToolParams>
      }
    />
  )
}
