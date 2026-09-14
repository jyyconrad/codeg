"use client"

import { useCallback, useMemo, useState } from "react"
import { useTranslations } from "next-intl"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { ParamPanel } from "./crypto-fields"
import {
  estimatePasswordStrength,
  formatPasswordStrength,
} from "./password-strength.core"

export default function PasswordStrengthTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const result = useMemo(() => {
    if (!input) return ""
    return formatPasswordStrength(estimatePasswordStrength(input))
  }, [input])

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      inputLabel="Password"
      result={result}
      cryptoFooter
      example="Tr0ub4dor&3"
      params={
        <ParamPanel title={t("params")}>
          <p className="text-xs text-muted-foreground">
            Estimated from length and character classes. Nothing is uploaded or
            saved.
          </p>
        </ParamPanel>
      }
    />
  )
}
