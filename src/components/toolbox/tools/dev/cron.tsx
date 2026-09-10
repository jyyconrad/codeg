"use client"

import { useCallback, useState } from "react"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { CRON_MAX_COUNT, CRON_MIN_COUNT, listCronNextRuns } from "./cron.core"
import { listToolTimezones } from "./time-zones"
import { ToolNumber, ToolParams, ToolSelect } from "./tool-controls"

export default function CronTool() {
  const [input, setInput] = useState("")
  const [timezone, setTimezone] = useState("UTC")
  const [count, setCount] = useState(5)
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const computed = listCronNextRuns(input, { timezone, count })

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      example="0 9 * * 1-5"
      result={computed.ok ? computed.output : ""}
      error={computed.ok ? null : computed.error}
      downloadFilename="cron-runs.txt"
      params={
        <ToolParams>
          <ToolSelect
            label="Time zone"
            value={timezone}
            onChange={setTimezone}
            options={listToolTimezones()}
          />
          <ToolNumber
            label="Next runs"
            value={count}
            min={CRON_MIN_COUNT}
            max={CRON_MAX_COUNT}
            onChange={setCount}
          />
        </ToolParams>
      }
    />
  )
}
