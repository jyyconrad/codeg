"use client"

import { useCallback, useEffect, useState } from "react"
import { useTranslations } from "next-intl"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Button } from "@/components/ui/button"
import { toErrorMessage } from "@/lib/app-error"
import { isLocalDesktop } from "@/lib/platform"
import {
  toolboxParseCert,
  toolboxRustAvailable,
  type ToolboxCertView,
} from "@/lib/toolbox-api"
import { ParamPanel, Warn } from "./crypto-fields"

function formatCert(view: ToolboxCertView): string {
  const san = view.san.length > 0 ? view.san.join("\n  ") : "(none)"
  return [
    `Subject:\n  ${view.subject}`,
    `Issuer:\n  ${view.issuer}`,
    `Serial:\n  ${view.serial}`,
    `Not before:\n  ${view.notBefore}`,
    `Not after:\n  ${view.notAfter}`,
    `SAN:\n  ${san}`,
    `SHA-256 fingerprint:\n  ${view.fingerprintSha256}`,
    `Signature OID:\n  ${view.signatureAlgorithm}`,
  ].join("\n\n")
}

export default function CertPemTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const [result, setResult] = useState("")
  const [error, setError] = useState<string | null>(null)

  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  useEffect(() => {
    let cancelled = false
    async function run() {
      if (!input.trim()) {
        setResult("")
        setError(null)
        return
      }
      if (!toolboxRustAvailable()) {
        setError("Certificate parsing needs the desktop app.")
        return
      }
      try {
        const view = await toolboxParseCert({ pem: input })
        if (!cancelled) {
          setResult(formatCert(view))
          setError(null)
        }
      } catch (err) {
        if (!cancelled) {
          setResult("")
          setError(toErrorMessage(err))
        }
      }
    }
    const timer = setTimeout(() => void run(), 200)
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [input])

  async function pickFile() {
    if (!toolboxRustAvailable()) {
      setError("Certificate parsing needs the desktop app.")
      return
    }
    const { open } = await import("@tauri-apps/plugin-dialog")
    const path = await open({
      multiple: false,
      title: "PEM certificate",
      filters: [{ name: "Certificates", extensions: ["pem", "crt", "cer"] }],
    })
    if (typeof path !== "string" || !path) return
    try {
      const view = await toolboxParseCert({ path })
      setResult(formatCert(view))
      setError(null)
    } catch (err) {
      setResult("")
      setError(toErrorMessage(err))
    }
  }

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      inputLabel="PEM"
      result={result}
      error={error}
      cryptoFooter
      example=""
      params={
        <div className="flex flex-col gap-2">
          <ParamPanel title={t("params")}>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={!isLocalDesktop()}
              onClick={() => void pickFile()}
            >
              Open PEM file…
            </Button>
          </ParamPanel>
          <Warn>Does not check CRL or OCSP. Local parse only.</Warn>
        </div>
      }
    />
  )
}
