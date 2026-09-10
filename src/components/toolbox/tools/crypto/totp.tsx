"use client"

import { useCallback, useEffect, useState } from "react"
import { useTranslations } from "next-intl"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Input } from "@/components/ui/input"
import { ParamField, ParamPanel, ParamSelect } from "./crypto-fields"
import { errorMessage } from "./encoding"
import {
  type TotpAlgorithm,
  generateTotp,
  parseTotpSecret,
  totpRemaining,
} from "./totp.core"

export default function TotpTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const [algorithm, setAlgorithm] = useState<TotpAlgorithm>("SHA-1")
  const [digits, setDigits] = useState(6)
  const [period, setPeriod] = useState(30)
  const [code, setCode] = useState("")
  const [remaining, setRemaining] = useState(30)
  const [error, setError] = useState<string | null>(null)

  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  useEffect(() => {
    let cancelled = false
    async function tick() {
      if (!input.trim()) {
        setCode("")
        setError(null)
        return
      }
      try {
        const secret = parseTotpSecret(input)
        const now = Math.floor(Date.now() / 1000)
        const next = await generateTotp({
          secret,
          unixSeconds: now,
          period,
          digits,
          algorithm,
        })
        if (!cancelled) {
          setCode(next)
          setRemaining(totpRemaining(now, period))
          setError(null)
        }
      } catch (err) {
        if (!cancelled) {
          setCode("")
          setError(errorMessage(err))
        }
      }
    }
    void tick()
    const id = window.setInterval(() => void tick(), 1000)
    return () => {
      cancelled = true
      window.clearInterval(id)
    }
  }, [algorithm, digits, input, period])

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      inputLabel="Base32 secret"
      result={code}
      error={error}
      cryptoFooter
      example="JBSWY3DPEHPK3PXP"
      params={
        <ParamPanel title={t("params")}>
          <ParamField label="Algorithm">
            <ParamSelect
              value={algorithm}
              onChange={(value) => setAlgorithm(value as TotpAlgorithm)}
              options={[
                { value: "SHA-1", label: "SHA-1" },
                { value: "SHA-256", label: "SHA-256" },
                { value: "SHA-512", label: "SHA-512" },
              ]}
            />
          </ParamField>
          <ParamField label="Digits">
            <ParamSelect
              value={String(digits)}
              onChange={(value) => setDigits(Number(value))}
              options={[
                { value: "6", label: "6" },
                { value: "8", label: "8" },
              ]}
            />
          </ParamField>
          <ParamField label="Period (seconds)">
            <Input
              type="number"
              min={1}
              value={period}
              onChange={(event) =>
                setPeriod(Number.parseInt(event.target.value, 10) || 30)
              }
            />
          </ParamField>
          <span className="pb-1 text-xs text-muted-foreground">
            Refreshes in {remaining}s. Secret stays in memory.
          </span>
        </ParamPanel>
      }
    />
  )
}
