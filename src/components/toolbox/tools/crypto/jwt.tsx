"use client"

import { useCallback, useEffect, useState } from "react"
import { useTranslations } from "next-intl"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Textarea } from "@/components/ui/textarea"
import { ParamField, ParamPanel } from "./crypto-fields"
import { errorMessage } from "./encoding"
import {
  decodeJwt,
  formatJwtDecode,
  verifyJwt,
  type JwtDecodeResult,
  type JwtVerifyResult,
} from "./jwt"

const EXAMPLE =
  "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c"

export default function JwtTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const [key, setKey] = useState("")
  const [decoded, setDecoded] = useState<JwtDecodeResult | null>(null)
  const [verify, setVerify] = useState<JwtVerifyResult | null>(null)
  const [error, setError] = useState<string | null>(null)

  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  useEffect(() => {
    let cancelled = false
    async function run() {
      if (!input.trim()) {
        setDecoded(null)
        setVerify(null)
        setError(null)
        return
      }
      try {
        const next = decodeJwt(input)
        const check = await verifyJwt({
          token: input,
          decoded: next,
          secretOrPem: key,
        })
        if (!cancelled) {
          setDecoded(next)
          setVerify(check)
          setError(null)
        }
      } catch (err) {
        if (!cancelled) {
          setDecoded(null)
          setVerify(null)
          setError(errorMessage(err))
        }
      }
    }
    void run()
    return () => {
      cancelled = true
    }
  }, [input, key])

  const result = decoded ? formatJwtDecode(decoded) : ""

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      inputLabel="Token"
      result={result}
      error={error}
      cryptoFooter
      example={EXAMPLE}
      resultSlot={
        <div className="flex min-h-[12rem] flex-1 flex-col gap-2">
          <div className="grid min-h-0 flex-1 grid-cols-1 gap-2 md:grid-cols-2">
            <label className="flex min-h-0 flex-col gap-1">
              <span className="text-xs font-medium text-muted-foreground">
                Decode
              </span>
              <Textarea
                readOnly
                value={error ? "" : result}
                className="min-h-[10rem] flex-1 font-mono text-sm"
              />
            </label>
            <label className="flex min-h-0 flex-col gap-1">
              <span className="text-xs font-medium text-muted-foreground">
                Verify
              </span>
              <Textarea
                readOnly
                value={
                  verify
                    ? [
                        verify.attempted
                          ? verify.verified
                            ? "Signature: verified"
                            : "Signature: not verified"
                          : "Signature: not checked",
                        `alg: ${verify.alg || "(missing)"}`,
                        verify.reason,
                      ].join("\n")
                    : ""
                }
                className="min-h-[10rem] flex-1 font-mono text-sm"
              />
            </label>
          </div>
        </div>
      }
      params={
        <ParamPanel title={t("params")}>
          <ParamField label="HMAC secret or RSA public PEM" className="w-full">
            <Textarea
              value={key}
              onChange={(event) => setKey(event.target.value)}
              className="min-h-20 font-mono text-sm"
              autoComplete="off"
            />
          </ParamField>
        </ParamPanel>
      }
    />
  )
}
