"use client"

import { useCallback, useMemo, useState } from "react"
import { useTranslations } from "next-intl"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Textarea } from "@/components/ui/textarea"
import {
  EncodingParams,
  EncodingSelect,
} from "@/components/toolbox/tools/encoding/encoding-params"
import { tryCodec } from "@/components/toolbox/tools/encoding/logic/error"
import {
  decodeUrl,
  encodeUrl,
  tryParseUrl,
  type UrlParts,
  type UrlVariant,
} from "@/components/toolbox/tools/encoding/logic/url-codec"

const EXAMPLE = "https://example.com/search?q=hello world&lang=zh-CN"
const MODE_OPTIONS = [
  { value: "encode", label: "encode" },
  { value: "decode", label: "decode" },
] as const

export default function UrlCodecTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const [mode, setMode] = useState<"encode" | "decode">("encode")
  const [variant, setVariant] = useState<UrlVariant>("uri")
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const { result, error } = useMemo(() => {
    return tryCodec(() =>
      mode === "encode" ? encodeUrl(input, variant) : decodeUrl(input, variant)
    )
  }, [input, mode, variant])

  const parts = tryParseUrl(input) ?? tryParseUrl(result)
  const variantOptions =
    mode === "encode"
      ? [
          { value: "uri", label: "encodeURI" },
          { value: "uri-component", label: "encodeURIComponent" },
        ]
      : [
          { value: "uri", label: "decodeURI" },
          { value: "uri-component", label: "decodeURIComponent" },
        ]

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      result={result}
      error={error}
      example={EXAMPLE}
      encodingNotice
      downloadFilename="url-codec.txt"
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
            value={variant}
            onChange={(value) => setVariant(value as UrlVariant)}
            options={variantOptions}
          />
        </EncodingParams>
      }
      resultSlot={
        <div className="flex min-h-0 flex-1 flex-col gap-2">
          <Textarea
            readOnly
            value={error ? "" : result}
            className="min-h-[12rem] flex-1 font-mono text-sm"
          />
          {parts ? <UrlPartsTable parts={parts} /> : null}
        </div>
      }
    />
  )
}

function UrlPartsTable({ parts }: { parts: UrlParts }) {
  return (
    <div className="overflow-auto rounded-xl border border-border text-xs">
      <table className="w-full text-left font-mono">
        <tbody>
          <UrlRow label="protocol" value={parts.protocol} />
          <UrlRow label="host" value={parts.host} />
          <UrlRow label="path" value={parts.path} />
          {parts.query.length === 0 ? (
            <UrlRow label="query" value="" />
          ) : (
            parts.query.map((param, index) => (
              <tr key={`${param.name}-${index}`}>
                <th className="px-2 py-1 font-medium text-muted-foreground">
                  {index === 0 ? "query" : ""}
                </th>
                <td className="px-2 py-1">
                  {param.name} = {param.value}
                </td>
              </tr>
            ))
          )}
        </tbody>
      </table>
    </div>
  )
}

function UrlRow({ label, value }: { label: string; value: string }) {
  return (
    <tr>
      <th className="px-2 py-1 font-medium text-muted-foreground">{label}</th>
      <td className="px-2 py-1">{value}</td>
    </tr>
  )
}
