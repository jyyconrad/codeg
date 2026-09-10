"use client"

import { useCallback, useEffect, useState } from "react"
import { useTranslations } from "next-intl"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { ParamPanel, Warn } from "./crypto-fields"
import { encodeUtf8, errorMessage } from "./encoding"
import {
  formatHashReport,
  hashBytes,
  readFileBytes,
  type HashReport,
} from "./hash"

export default function HashTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const [fileName, setFileName] = useState<string | null>(null)
  const [result, setResult] = useState("")
  const [error, setError] = useState<string | null>(null)

  const onInput = useCallback((value: string) => {
    setFileName(null)
    setInput(value)
  }, [])
  useToolPendingInput(onInput)

  useEffect(() => {
    let cancelled = false
    async function run() {
      if (fileName) return
      if (!input) {
        setResult("")
        setError(null)
        return
      }
      try {
        const report: HashReport = await hashBytes(encodeUtf8(input))
        if (!cancelled) {
          setResult(formatHashReport(report))
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
  }, [fileName, input])

  async function onFile(file: File | undefined) {
    if (!file) {
      setFileName(null)
      return
    }
    setFileName(file.name)
    setInput("")
    try {
      const bytes = await readFileBytes(file)
      const report = await hashBytes(bytes)
      setResult(formatHashReport(report))
      setError(null)
    } catch (err) {
      setResult("")
      setError(errorMessage(err))
    }
  }

  return (
    <ToolPageShell
      input={input}
      onInputChange={(value) => {
        setFileName(null)
        setInput(value)
      }}
      inputLabel="Text"
      result={result}
      error={error}
      cryptoFooter
      example="abc"
      params={
        <div className="flex flex-col gap-2">
          <ParamPanel title={t("params")}>
            <label className="flex flex-col gap-1 text-xs text-muted-foreground">
              Small file
              <input
                type="file"
                className="text-sm text-foreground"
                onChange={(event) => void onFile(event.target.files?.[0])}
              />
            </label>
            {fileName ? (
              <span className="pb-1 text-xs text-muted-foreground">
                Hashing file: {fileName}
              </span>
            ) : null}
          </ParamPanel>
          <Warn>MD5 / SHA-1: checksum only, not for passwords.</Warn>
        </div>
      }
    />
  )
}
