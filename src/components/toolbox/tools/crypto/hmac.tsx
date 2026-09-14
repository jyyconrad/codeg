"use client"

import { useCallback, useEffect, useState } from "react"
import { useTranslations } from "next-intl"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Input } from "@/components/ui/input"
import { ParamField, ParamPanel, ParamSelect } from "./crypto-fields"
import { type ByteEncoding, errorMessage } from "./encoding"
import {
  type HmacAlgorithm,
  type HmacOutputEncoding,
  hmacText,
} from "./hmac.core"

const ALGORITHMS: { value: HmacAlgorithm; label: string }[] = [
  { value: "SHA-256", label: "SHA-256" },
  { value: "SHA-512", label: "SHA-512" },
  { value: "SM3", label: "SM3" },
]

const ENCODINGS: { value: ByteEncoding; label: string }[] = [
  { value: "utf8", label: "UTF-8" },
  { value: "hex", label: "Hex" },
  { value: "base64", label: "Base64" },
]

export default function HmacTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const [key, setKey] = useState("")
  const [algorithm, setAlgorithm] = useState<HmacAlgorithm>("SHA-256")
  const [keyEncoding, setKeyEncoding] = useState<ByteEncoding>("utf8")
  const [messageEncoding, setMessageEncoding] = useState<ByteEncoding>("utf8")
  const [outputEncoding, setOutputEncoding] =
    useState<HmacOutputEncoding>("hex")
  const [result, setResult] = useState("")
  const [error, setError] = useState<string | null>(null)

  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  useEffect(() => {
    let cancelled = false
    async function run() {
      if (!input && !key) {
        setResult("")
        setError(null)
        return
      }
      try {
        const out = await hmacText({
          algorithm,
          key,
          keyEncoding,
          message: input,
          messageEncoding,
          outputEncoding,
        })
        if (!cancelled) {
          setResult(out)
          setError(null)
        }
      } catch (err) {
        if (!cancelled) {
          setResult("")
          setError(errorMessage(err))
        }
      }
    }
    void run()
    return () => {
      cancelled = true
    }
  }, [algorithm, input, key, keyEncoding, messageEncoding, outputEncoding])

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      inputLabel="Message"
      result={result}
      error={error}
      cryptoFooter
      example="Hi There"
      params={
        <ParamPanel title={t("params")}>
          <ParamField label="Algorithm">
            <ParamSelect
              value={algorithm}
              onChange={(value) => setAlgorithm(value as HmacAlgorithm)}
              options={ALGORITHMS}
            />
          </ParamField>
          <ParamField label="Key encoding">
            <ParamSelect
              value={keyEncoding}
              onChange={(value) => setKeyEncoding(value as ByteEncoding)}
              options={ENCODINGS}
            />
          </ParamField>
          <ParamField label="Message encoding">
            <ParamSelect
              value={messageEncoding}
              onChange={(value) => setMessageEncoding(value as ByteEncoding)}
              options={ENCODINGS}
            />
          </ParamField>
          <ParamField label="Output encoding">
            <ParamSelect
              value={outputEncoding}
              onChange={(value) =>
                setOutputEncoding(value as HmacOutputEncoding)
              }
              options={[
                { value: "hex", label: "Hex" },
                { value: "base64", label: "Base64" },
              ]}
            />
          </ParamField>
          <ParamField label="Key" className="min-w-[16rem] flex-1">
            <Input
              value={key}
              onChange={(event) => setKey(event.target.value)}
              className="font-mono"
              autoComplete="off"
            />
          </ParamField>
        </ParamPanel>
      }
    />
  )
}
