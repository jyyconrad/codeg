"use client"

import { useCallback, useState } from "react"
import { useTranslations } from "next-intl"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { toErrorMessage } from "@/lib/app-error"
import { isLocalDesktop } from "@/lib/platform"
import {
  toolboxBcryptHash,
  toolboxBcryptVerify,
  toolboxCancelJob,
  toolboxRustAvailable,
} from "@/lib/toolbox-api"
import { ParamField, ParamPanel, ParamSelect, Warn } from "./crypto-fields"

const COST_MIN = 4
const COST_MAX = 14

export default function BcryptTool() {
  const t = useTranslations("Toolbox")
  const [password, setPassword] = useState("")
  const [hash, setHash] = useState("")
  const [cost, setCost] = useState("10")
  const [mode, setMode] = useState<"hash" | "verify">("hash")
  const [result, setResult] = useState("")
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [jobId, setJobId] = useState<string | null>(null)

  const onInput = useCallback((value: string) => setPassword(value), [])
  useToolPendingInput(onInput)

  async function run() {
    if (!toolboxRustAvailable()) {
      setError("bcrypt needs the desktop app.")
      return
    }
    setBusy(true)
    setError(null)
    const id = crypto.randomUUID()
    setJobId(id)
    try {
      if (mode === "hash") {
        const out = await toolboxBcryptHash(password, Number(cost), id)
        setResult(out)
        setHash(out)
      } else {
        const ok = await toolboxBcryptVerify(password, hash)
        setResult(ok ? "Match" : "No match")
      }
    } catch (err) {
      const message = toErrorMessage(err)
      if (message !== "Cancelled") setError(message)
    } finally {
      setBusy(false)
      setJobId(null)
    }
  }

  return (
    <ToolPageShell
      input={password}
      onInputChange={setPassword}
      inputLabel="Password"
      inputSlot={
        <Input
          type="password"
          value={password}
          onChange={(event) => setPassword(event.target.value)}
          autoComplete="off"
          className="font-mono"
        />
      }
      result={result}
      error={error}
      cryptoFooter
      example=""
      onExample={() => {
        setPassword("correct horse battery staple")
        setMode("hash")
        setCost("10")
      }}
      params={
        <div className="flex flex-col gap-2">
          <ParamPanel title={t("params")}>
            <ParamField label="Mode">
              <ParamSelect
                value={mode}
                onChange={(value) => setMode(value as "hash" | "verify")}
                options={[
                  { value: "hash", label: "Hash" },
                  { value: "verify", label: "Verify" },
                ]}
              />
            </ParamField>
            {mode === "hash" ? (
              <ParamField label="Cost">
                <ParamSelect
                  value={cost}
                  onChange={setCost}
                  options={Array.from(
                    { length: COST_MAX - COST_MIN + 1 },
                    (_, i) => {
                      const n = String(i + COST_MIN)
                      return { value: n, label: n }
                    }
                  )}
                />
              </ParamField>
            ) : (
              <ParamField label="Hash" className="min-w-[16rem] flex-1">
                <Input
                  value={hash}
                  onChange={(event) => setHash(event.target.value)}
                  className="font-mono"
                  autoComplete="off"
                />
              </ParamField>
            )}
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={!isLocalDesktop() || busy || !password}
              onClick={() => void run()}
            >
              {busy ? "Working…" : mode === "hash" ? "Hash" : "Verify"}
            </Button>
            {busy && jobId ? (
              <Button
                type="button"
                variant="outline"
                size="xs"
                onClick={() => void toolboxCancelJob(jobId)}
              >
                Cancel
              </Button>
            ) : null}
          </ParamPanel>
          <Warn>
            Password is not stored. Cost {COST_MIN}–{COST_MAX}; default 10.
          </Warn>
        </div>
      }
    />
  )
}
