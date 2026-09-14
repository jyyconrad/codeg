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
  decodeHex,
  encodeHex,
  type HexCharset,
} from "@/components/toolbox/tools/encoding/logic/hex-string"

const EXAMPLE = "Hello, 世界"
const MODE_OPTIONS = [
  { value: "encode", label: "encode" },
  { value: "decode", label: "decode" },
] as const
const CHARSET_OPTIONS = [
  { value: "utf-8", label: "UTF-8" },
  { value: "latin1", label: "Latin-1" },
] as const

export default function HexStringTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const [mode, setMode] = useState<"encode" | "decode">("encode")
  const [charset, setCharset] = useState<HexCharset>("utf-8")
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const { result, error } = useMemo(() => {
    return tryCodec(() =>
      mode === "encode" ? encodeHex(input, charset) : decodeHex(input, charset)
    )
  }, [charset, input, mode])

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      result={result}
      error={error}
      example={EXAMPLE}
      encodingNotice
      downloadFilename="hex-string.txt"
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
            value={charset}
            onChange={(value) => setCharset(value as HexCharset)}
            options={CHARSET_OPTIONS}
          />
        </EncodingParams>
      }
    />
  )
}
