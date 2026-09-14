"use client"

import { useCallback, useState } from "react"
import { Button } from "@/components/ui/button"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import {
  generateUuidBatch,
  UUID_MAX_COUNT,
  UUID_MIN_COUNT,
  type UuidVersion,
} from "./uuid.core"
import { ToolNumber, ToolParams, ToolSelect } from "./tool-controls"

export default function UuidTool() {
  const [input, setInput] = useState("")
  const [version, setVersion] = useState<UuidVersion>("v4")
  const [count, setCount] = useState(5)
  const [error, setError] = useState<string | null>(null)
  const onInput = useCallback((value: string) => {
    setInput(value)
    setError(null)
  }, [])
  useToolPendingInput(onInput)

  function generate(nextVersion = version, nextCount = count) {
    const result = generateUuidBatch(nextVersion, nextCount)
    if (result.ok) {
      setInput(result.output)
      setError(null)
    } else {
      setError(result.error)
    }
  }

  return (
    <ToolPageShell
      input={input}
      onInputChange={onInput}
      result={input}
      error={error}
      downloadFilename="uuids.txt"
      onExample={() => {
        setVersion("v4")
        setCount(5)
        generate("v4", 5)
      }}
      params={
        <ToolParams>
          <ToolSelect
            label="Version"
            value={version}
            onChange={(value) => setVersion(value as UuidVersion)}
            options={[
              { value: "v4", label: "UUID v4" },
              { value: "v7", label: "UUID v7" },
            ]}
          />
          <ToolNumber
            label="Count"
            value={count}
            min={UUID_MIN_COUNT}
            max={UUID_MAX_COUNT}
            onChange={setCount}
          />
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => generate()}
          >
            Generate
          </Button>
        </ToolParams>
      }
    />
  )
}
