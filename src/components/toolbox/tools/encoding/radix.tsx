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
  convertRadix,
  RADIX_MAX,
  RADIX_MIN,
} from "@/components/toolbox/tools/encoding/logic/radix"

const EXAMPLE = "255"
const RADIX_OPTIONS = Array.from(
  { length: RADIX_MAX - RADIX_MIN + 1 },
  (_, index) => {
    const radix = index + RADIX_MIN
    return { value: String(radix), label: String(radix) }
  }
)

export default function RadixTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const [fromRadix, setFromRadix] = useState(10)
  const [toRadix, setToRadix] = useState(16)
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const { result, error } = useMemo(() => {
    return tryCodec(() => convertRadix(input, fromRadix, toRadix))
  }, [fromRadix, input, toRadix])

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      result={result}
      error={error}
      example={EXAMPLE}
      encodingNotice
      downloadFilename="radix.txt"
      params={
        <EncodingParams>
          <EncodingSelect
            aria-label={t("input")}
            value={String(fromRadix)}
            onChange={(value) => setFromRadix(Number(value))}
            options={RADIX_OPTIONS}
          />
          <span className="text-xs text-muted-foreground">→</span>
          <EncodingSelect
            aria-label={t("output")}
            value={String(toRadix)}
            onChange={(value) => setToRadix(Number(value))}
            options={RADIX_OPTIONS}
          />
        </EncodingParams>
      }
    />
  )
}
