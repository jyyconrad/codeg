"use client"

import { useCallback, useMemo, useState } from "react"
import { ToolboxSelect } from "@/components/toolbox/toolbox-controls"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import { dedupeSortLines, type LineSortMode } from "./line-dedupe-sort-core"

const EXAMPLE = "Banana\napple\n  banana  \nApple\ncherry\napple"

export default function LineDedupeSortTool() {
  const [input, setInput] = useState("")
  const [ignoreCase, setIgnoreCase] = useState(true)
  const [trimWhitespace, setTrimWhitespace] = useState(true)
  const [sort, setSort] = useState<LineSortMode>("keep")
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const result = useMemo(
    () =>
      dedupeSortLines(input, {
        ignoreCase,
        trimWhitespace,
        sort,
      }),
    [input, ignoreCase, trimWhitespace, sort]
  )

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      example={EXAMPLE}
      result={result}
      downloadFilename="lines.txt"
      params={
        <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
          <Label className="flex items-center gap-2 text-xs font-normal text-muted-foreground">
            <Checkbox
              checked={ignoreCase}
              onCheckedChange={(value) => setIgnoreCase(value === true)}
            />
            Ignore case
          </Label>
          <Label className="flex items-center gap-2 text-xs font-normal text-muted-foreground">
            <Checkbox
              checked={trimWhitespace}
              onCheckedChange={(value) => setTrimWhitespace(value === true)}
            />
            Trim whitespace
          </Label>
          <ToolboxSelect
            label="Sort"
            value={sort}
            onChange={(value) => setSort(value as LineSortMode)}
            options={[
              { value: "keep", label: "Keep order" },
              { value: "asc", label: "A–Z" },
              { value: "desc", label: "Z–A" },
            ]}
          />
        </div>
      }
    />
  )
}
