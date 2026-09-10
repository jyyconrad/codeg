"use client"

import { useCallback, useState } from "react"
import { useTranslations } from "next-intl"
import { ToolboxCheckbox } from "@/components/toolbox/toolbox-controls"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ParamField, ParamPanel } from "./crypto-fields"
import { errorMessage } from "./encoding"
import { type PasswordSetId, generatePassword } from "./password-gen.core"

const SETS: { id: PasswordSetId; label: string }[] = [
  { id: "lower", label: "a-z" },
  { id: "upper", label: "A-Z" },
  { id: "digits", label: "0-9" },
  { id: "symbols", label: "Symbols" },
]

export default function PasswordGenTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const [length, setLength] = useState(16)
  const [sets, setSets] = useState<PasswordSetId[]>([
    "lower",
    "upper",
    "digits",
  ])
  const [excludeSimilar, setExcludeSimilar] = useState(false)
  const [password, setPassword] = useState("")
  const [error, setError] = useState<string | null>(null)

  const onInput = useCallback((value: string) => {
    setInput(value)
    if (!value) setPassword("")
  }, [])
  useToolPendingInput(onInput)

  function generate() {
    try {
      const next = generatePassword({ length, sets, excludeSimilar })
      setPassword(next)
      setError(null)
    } catch (err) {
      setPassword("")
      setError(errorMessage(err))
    }
  }

  function toggleSet(id: PasswordSetId, on: boolean) {
    setSets((current) =>
      on ? [...current, id] : current.filter((item) => item !== id)
    )
  }

  return (
    <ToolPageShell
      input={input}
      onInputChange={(value) => {
        setInput(value)
        if (!value) setPassword("")
      }}
      result={password}
      error={error}
      cryptoFooter
      onExample={generate}
      inputSlot={
        <div className="flex min-h-[12rem] flex-1 flex-col justify-center gap-2 rounded-xl border border-border bg-input/30 px-3 py-3 text-sm text-muted-foreground">
          <p>Generated on this device with crypto.getRandomValues.</p>
          <p>Nothing is stored. Copy or send the result if you need it.</p>
        </div>
      }
      params={
        <ParamPanel title={t("params")}>
          <ParamField label="Length">
            <Input
              type="number"
              min={1}
              max={256}
              value={length}
              onChange={(event) =>
                setLength(Number.parseInt(event.target.value, 10) || 1)
              }
            />
          </ParamField>
          {SETS.map((set) => (
            <ToolboxCheckbox
              key={set.id}
              label={set.label}
              checked={sets.includes(set.id)}
              onChange={(checked) => toggleSet(set.id, checked)}
            />
          ))}
          <ToolboxCheckbox
            label="Exclude 0/O/1/I/l"
            checked={excludeSimilar}
            onChange={setExcludeSimilar}
          />
          <Button type="button" size="sm" onClick={generate}>
            Generate
          </Button>
        </ParamPanel>
      }
    />
  )
}
