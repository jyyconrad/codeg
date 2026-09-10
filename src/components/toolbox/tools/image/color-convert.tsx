"use client"

import { useCallback, useMemo, useState } from "react"
import { useTranslations } from "next-intl"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Input } from "@/components/ui/input"
import {
  colorError,
  formatHex,
  formatHsl,
  formatRgb,
  parseColor,
  rgbToHsl,
  toCssColor,
  type Rgba,
} from "./color"

const EXAMPLE = "#3388FF80"

function ColorSwatch({ color }: { color: Rgba }) {
  return (
    <div className="relative size-16 overflow-hidden rounded-md border">
      <div
        className="absolute inset-0"
        style={{
          backgroundImage:
            "linear-gradient(45deg, #d4d4d4 25%, transparent 25%), linear-gradient(-45deg, #d4d4d4 25%, transparent 25%), linear-gradient(45deg, transparent 75%, #d4d4d4 75%), linear-gradient(-45deg, transparent 75%, #d4d4d4 75%)",
          backgroundPosition: "0 0, 0 6px, 6px -6px, -6px 0",
          backgroundSize: "12px 12px",
        }}
      />
      <div
        className="absolute inset-0"
        style={{ background: toCssColor(color) }}
      />
    </div>
  )
}

export default function ColorConvertTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const parsed = useMemo(() => parseColor(input), [input])
  const error = colorError(input)
  const hsl = parsed ? rgbToHsl(parsed.rgba) : null
  const result = parsed
    ? [formatHex(parsed.rgba), formatRgb(parsed.rgba), formatHsl(hsl!)].join(
        "\n"
      )
    : ""

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      inputLabel={t("input")}
      params={
        <p className="text-xs text-muted-foreground">
          {t("params")}: HEX / RGB / HSL
        </p>
      }
      result={result}
      error={error}
      example={EXAMPLE}
      resultSlot={
        parsed ? (
          <div className="flex min-h-[12rem] flex-1 flex-col gap-3">
            <ColorSwatch color={parsed.rgba} />
            <Input readOnly value={formatHex(parsed.rgba)} />
            <Input readOnly value={formatRgb(parsed.rgba)} />
            <Input readOnly value={formatHsl(hsl!)} />
          </div>
        ) : undefined
      }
    />
  )
}
