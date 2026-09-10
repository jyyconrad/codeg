"use client"

import { useCallback, useMemo, useRef, useState } from "react"
import { useTranslations } from "next-intl"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import {
  EncodingParams,
  EncodingSelect,
} from "@/components/toolbox/tools/encoding/encoding-params"
import {
  decodeBase64,
  encodeBase64,
  encodeBase64Bytes,
} from "@/components/toolbox/tools/encoding/logic/base64"
import { tryCodec } from "@/components/toolbox/tools/encoding/logic/error"

const EXAMPLE = "Hello, 世界"
const MODE_OPTIONS = [
  { value: "encode", label: "encode" },
  { value: "decode", label: "decode" },
] as const

function readFileAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      const buffer = reader.result
      if (!(buffer instanceof ArrayBuffer)) {
        reject(new Error("Invalid file"))
        return
      }
      resolve(encodeBase64Bytes(new Uint8Array(buffer)))
    }
    reader.onerror = () => {
      reject(reader.error ?? new Error("Invalid file"))
    }
    reader.readAsArrayBuffer(file)
  })
}

export default function Base64Tool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const [mode, setMode] = useState<"encode" | "decode">("encode")
  const [fileBase64, setFileBase64] = useState<string | null>(null)
  const [fileError, setFileError] = useState<string | null>(null)
  const fileGen = useRef(0)

  const onInput = useCallback((value: string) => {
    setFileBase64(null)
    setFileError(null)
    setInput(value)
  }, [])
  useToolPendingInput(onInput)

  const { result, error } = useMemo(() => {
    if (fileError) return { result: "", error: fileError }
    if (fileBase64 != null) return { result: fileBase64, error: null }
    return tryCodec(() =>
      mode === "encode" ? encodeBase64(input) : decodeBase64(input)
    )
  }, [fileBase64, fileError, input, mode])

  async function onFile(file: File | undefined) {
    const gen = ++fileGen.current
    setFileBase64(null)
    setFileError(null)
    if (!file) return
    try {
      const encoded = await readFileAsBase64(file)
      if (gen !== fileGen.current) return
      setMode("encode")
      setInput("")
      setFileBase64(encoded)
    } catch (caught) {
      if (gen !== fileGen.current) return
      setFileError(caught instanceof Error ? caught.message : String(caught))
    }
  }

  return (
    <ToolPageShell
      input={input}
      onInputChange={onInput}
      result={result}
      error={error}
      example={EXAMPLE}
      encodingNotice
      downloadFilename="base64.txt"
      params={
        <EncodingParams>
          <EncodingSelect
            aria-label={t("categories.encoding")}
            value={mode}
            onChange={(value) => {
              setFileBase64(null)
              setFileError(null)
              setMode(value as "encode" | "decode")
            }}
            options={MODE_OPTIONS}
          />
          <input
            type="file"
            aria-label={t("input")}
            className="max-w-xs text-xs text-muted-foreground file:mr-2 file:h-8 file:rounded-full file:border file:border-border file:bg-input/30 file:px-3 file:text-xs file:text-foreground"
            onChange={(event) => {
              const file = event.target.files?.[0]
              event.target.value = ""
              void onFile(file)
            }}
          />
        </EncodingParams>
      }
    />
  )
}
