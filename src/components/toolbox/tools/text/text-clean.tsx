"use client"

import { useCallback, useMemo, useState } from "react"
import { ToolboxSelect } from "@/components/toolbox/toolbox-controls"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import { cleanText, type PunctMode, type WidthMode } from "./text-clean-core"

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
          <ToolboxSelect
            label="Width"
            value={width}
            onChange={(value) => setWidth(value as WidthMode)}
            options={[
              { value: "off", label: "Off" },
              { value: "full-to-half", label: "Full → half" },
              { value: "half-to-full", label: "Half → full" },
            ]}
          />
          <ToolboxSelect
            label="Punctuation"
            value={punct}
            onChange={(value) => setPunct(value as PunctMode)}
            options={[
              { value: "off", label: "Off" },
              { value: "cjk-to-ascii", label: "CJK → ASCII" },
              { value: "ascii-to-cjk", label: "ASCII → CJK" },
            ]}
          />
        </div>
      }
    />
  )
}
