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
  escapeHtml,
  unescapeHtml,
} from "@/components/toolbox/tools/encoding/logic/html-entities"

const EXAMPLE = `<div class="x">A & B's "C"</div>`
const MODE_OPTIONS = [
  { value: "escape", label: "escape" },
  { value: "unescape", label: "unescape" },
] as const

export default function HtmlEntitiesTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const [mode, setMode] = useState<"escape" | "unescape">("escape")
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const { result, error } = useMemo(() => {
    return tryCodec(() =>
      mode === "escape" ? escapeHtml(input) : unescapeHtml(input)
    )
  }, [input, mode])

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      result={result}
      error={error}
      example={EXAMPLE}
      encodingNotice
      downloadFilename="html-entities.txt"
      params={
        <EncodingParams>
          <EncodingSelect
            aria-label={t("categories.encoding")}
            value={mode}
            onChange={(value) => setMode(value as "escape" | "unescape")}
            options={MODE_OPTIONS}
          />
        </EncodingParams>
      }
    />
  )
}
