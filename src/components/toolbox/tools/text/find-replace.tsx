"use client"

import { useCallback, useMemo, useState } from "react"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import { FIND_REPLACE_PREVIEW_LIMIT, runFindReplace } from "./find-replace-core"

const EXAMPLE_INPUT = "The rain in Spain stays mainly in the plain."
const EXAMPLE_FIND = "ain"
const EXAMPLE_REPLACE = "oat"

export default function FindReplaceTool() {
  const [input, setInput] = useState("")
  const [find, setFind] = useState("")
  const [replace, setReplace] = useState("")
  const [useRegex, setUseRegex] = useState(false)
  const [ignoreCase, setIgnoreCase] = useState(false)
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const { text, hits, error } = useMemo(
    () =>
      runFindReplace(input, {
        find,
        replace,
        useRegex,
        ignoreCase,
      }),
    [input, find, replace, useRegex, ignoreCase]
  )

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      onExample={() => {
        setInput(EXAMPLE_INPUT)
        setFind(EXAMPLE_FIND)
        setReplace(EXAMPLE_REPLACE)
      }}
      result={error ? "" : text}
      error={error}
      downloadFilename="replaced.txt"
      params={
        <div className="flex flex-col gap-2">
          <div className="flex flex-wrap items-end gap-2">
            <label className="flex min-w-[12rem] flex-1 flex-col gap-1 text-xs text-muted-foreground">
              Find
              <Input
                value={find}
                onChange={(event) => setFind(event.target.value)}
                className="font-mono"
              />
            </label>
            <label className="flex min-w-[12rem] flex-1 flex-col gap-1 text-xs text-muted-foreground">
              Replace
              <Input
                value={replace}
                onChange={(event) => setReplace(event.target.value)}
                className="font-mono"
              />
            </label>
          </div>
          <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
            <Label className="flex items-center gap-2 text-xs font-normal text-muted-foreground">
              <Checkbox
                checked={useRegex}
                onCheckedChange={(value) => setUseRegex(value === true)}
              />
              Regular expression
            </Label>
            <Label className="flex items-center gap-2 text-xs font-normal text-muted-foreground">
              <Checkbox
                checked={ignoreCase}
                onCheckedChange={(value) => setIgnoreCase(value === true)}
              />
              Ignore case
            </Label>
            <span className="text-xs text-muted-foreground">
              {hits.length}
              {hits.length >= FIND_REPLACE_PREVIEW_LIMIT ? "+" : ""} matches
            </span>
          </div>
        </div>
      }
      resultSlot={
        <div className="flex min-h-0 flex-1 flex-col gap-2">
          <ul className="max-h-28 overflow-auto rounded-xl border border-border bg-input/30 p-2 font-mono text-xs">
            {hits.length === 0 ? (
              <li className="text-muted-foreground">No matches</li>
            ) : (
              hits.map((hit, index) => (
                <li key={`${hit.index}-${index}`}>
                  L{hit.line}:{hit.column} {JSON.stringify(hit.match)}
                  {" → "}
                  {JSON.stringify(hit.replacement)}
                </li>
              ))
            )}
          </ul>
          <Textarea
            readOnly
            value={error ? "" : text}
            className="min-h-[8rem] flex-1 font-mono text-sm"
          />
        </div>
      }
    />
  )
}
