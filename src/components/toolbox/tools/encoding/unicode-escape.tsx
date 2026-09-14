"use client"

import { useCallback, useMemo, useState } from "react"
import { useTranslations } from "next-intl"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import {
  EncodingParams,
  EncodingSelect,
} from "@/components/toolbox/tools/encoding/encoding-params"
import { tryCodec } from "@/components/toolbox/tools/encoding/logic/error"
import {
  decodeUnicode,
  encodeUnicode,
  type UnicodeStyle,
} from "@/components/toolbox/tools/encoding/logic/unicode-escape"

const EXAMPLE = "Hello, 世界"
const MODE_OPTIONS = [
  { value: "encode", label: "encode" },
  { value: "decode", label: "decode" },
] as const
const STYLE_OPTIONS = [
  { value: "unicode", label: "\\uXXXX" },
  { value: "json", label: "JSON" },
] as const

export default function UnicodeEscapeTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const [mode, setMode] = useState<"encode" | "decode">("encode")
  const [style, setStyle] = useState<UnicodeStyle>("unicode")
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const { result, error } = useMemo(() => {
    return tryCodec(() =>
      mode === "encode"
        ? encodeUnicode(input, style)
        : decodeUnicode(input, style)
    )
  }, [input, mode, style])

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      result={result}
      error={error}
      example={EXAMPLE}
      encodingNotice
      downloadFilename="unicode-escape.txt"
      params={
        <EncodingParams>
          <EncodingSelect
            aria-label={t("categories.encoding")}
            value={mode}
            onChange={(value) => setMode(value as "encode" | "decode")}
            options={MODE_OPTIONS}
          />
          <EncodingSelect
            aria-label={t("params")}
            value={style}
            onChange={(value) => setStyle(value as UnicodeStyle)}
            options={STYLE_OPTIONS}
          />
        </EncodingParams>
      }
    />
  )
}
