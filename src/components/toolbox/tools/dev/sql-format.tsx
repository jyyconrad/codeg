"use client"

import { useCallback, useState } from "react"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { formatSql, SQL_DIALECTS, type SqlFormatMode } from "./sql-format.core"
import { ToolParams, ToolSelect } from "./tool-controls"

const EXAMPLE = "select id, name from users where active = 1 order by name;"

export default function SqlFormatTool() {
  const [input, setInput] = useState("")
  const [mode, setMode] = useState<SqlFormatMode>("pretty")
  const [language, setLanguage] = useState("sql")
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const computed = formatSql(input, { mode, language })

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      example={EXAMPLE}
      result={computed.ok ? computed.output : ""}
      error={computed.ok ? null : computed.error}
      downloadFilename="formatted.sql"
      params={
        <ToolParams>
          <ToolSelect
            label="Mode"
            value={mode}
            onChange={(value) => setMode(value as SqlFormatMode)}
            options={[
              { value: "pretty", label: "Pretty" },
              { value: "minify", label: "Minify" },
            ]}
          />
          <ToolSelect
            label="Dialect"
            value={language}
            onChange={setLanguage}
            options={SQL_DIALECTS}
          />
        </ToolParams>
      }
    />
  )
}
