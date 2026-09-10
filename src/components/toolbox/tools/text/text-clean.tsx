"use client"

import { useCallback, useMemo, useState } from "react"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import { cleanText, type PunctMode, type WidthMode } from "./text-clean-core"

const SELECT_CLASS =
  "h-8 rounded-full border border-border bg-input/30 px-2 text-xs text-foreground"

const EXAMPLE = "  Hello，　Ｗｏｒｌｄ！  \r\n\r\n\r\nFullwidth：ＡＢＣ  "

export default function TextCleanTool() {
  const [input, setInput] = useState("")
  const [trim, setTrim] = useState(true)
  const [dropBlankLines, setDropBlankLines] = useState(true)
  const [normalizeNewlines, setNormalizeNewlines] = useState(true)
  const [width, setWidth] = useState<WidthMode>("off")
  const [punct, setPunct] = useState<PunctMode>("off")
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const result = useMemo(
    () =>
      cleanText(input, {
        trim,
        dropBlankLines,
        normalizeNewlines,
        width,
        punct,
      }),
    [input, trim, dropBlankLines, normalizeNewlines, width, punct]
  )

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      example={EXAMPLE}
      result={result}
      downloadFilename="cleaned.txt"
      params={
        <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
          <Label className="flex items-center gap-2 text-xs font-normal text-muted-foreground">
            <Checkbox
              checked={trim}
              onCheckedChange={(value) => setTrim(value === true)}
            />
            Trim
          </Label>
          <Label className="flex items-center gap-2 text-xs font-normal text-muted-foreground">
            <Checkbox
              checked={dropBlankLines}
              onCheckedChange={(value) => setDropBlankLines(value === true)}
            />
            Drop blank lines
          </Label>
          <Label className="flex items-center gap-2 text-xs font-normal text-muted-foreground">
            <Checkbox
              checked={normalizeNewlines}
              onCheckedChange={(value) => setNormalizeNewlines(value === true)}
            />
            Normalize newlines
          </Label>
          <label className="flex items-center gap-2 text-xs text-muted-foreground">
            Width
            <select
              className={SELECT_CLASS}
              value={width}
              onChange={(event) => setWidth(event.target.value as WidthMode)}
            >
              <option value="off">Off</option>
              <option value="full-to-half">Full → half</option>
              <option value="half-to-full">Half → full</option>
            </select>
          </label>
          <label className="flex items-center gap-2 text-xs text-muted-foreground">
            Punctuation
            <select
              className={SELECT_CLASS}
              value={punct}
              onChange={(event) => setPunct(event.target.value as PunctMode)}
            >
              <option value="off">Off</option>
              <option value="cjk-to-ascii">CJK → ASCII</option>
              <option value="ascii-to-cjk">ASCII → CJK</option>
            </select>
          </label>
        </div>
      }
    />
  )
}
