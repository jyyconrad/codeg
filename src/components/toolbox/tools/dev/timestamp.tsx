"use client"

import { useCallback, useState } from "react"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { convertTimestamp, type TimestampUnit } from "./timestamp.core"
import { listToolTimezones } from "./time-zones"
import { ToolParams, ToolSelect } from "./tool-controls"

export default function TimestampTool() {
  const [input, setInput] = useState("")
  const [timeZone, setTimeZone] = useState("UTC")
  const [unit, setUnit] = useState<TimestampUnit>("auto")
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const computed = convertTimestamp(input, { timeZone, unit })

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      onExample={() => setInput(String(Math.floor(Date.now() / 1000)))}
      result={computed.ok ? computed.output : ""}
      error={computed.ok ? null : computed.error}
      downloadFilename="timestamp.txt"
      params={
        <ToolParams>
          <ToolSelect
            label="Time zone"
            value={timeZone}
            onChange={setTimeZone}
            options={listToolTimezones()}
          />
          <ToolSelect
            label="Unit"
            value={unit}
            onChange={(value) => setUnit(value as TimestampUnit)}
            options={[
              { value: "auto", label: "Auto" },
              { value: "seconds", label: "Seconds" },
              { value: "milliseconds", label: "Milliseconds" },
            ]}
          />
        </ToolParams>
      }
    />
  )
}
