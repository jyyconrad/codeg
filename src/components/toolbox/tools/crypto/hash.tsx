"use client"

import { useCallback, useEffect, useState } from "react"
import { useTranslations } from "next-intl"
import { FileJobBar } from "@/components/toolbox/file-job-bar"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Button } from "@/components/ui/button"
import { toErrorMessage } from "@/lib/app-error"
import { isLocalDesktop } from "@/lib/platform"
import {
  formatToolboxHashReport,
  listenToolboxProgress,
  toolboxCancelJob,
  toolboxHashFile,
  toolboxRustAvailable,
  type ToolboxProgress,
} from "@/lib/toolbox-api"
import { ParamPanel, Warn } from "./crypto-fields"
import { encodeUtf8, errorMessage } from "./encoding"
import {
  formatHashReport,
  hashBytes,
  readFileBytes,
  SMALL_FILE_MAX_BYTES,
  type HashReport,
} from "./hash"

export default function HashTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const [fileName, setFileName] = useState<string | null>(null)
  const [result, setResult] = useState("")
  const [error, setError] = useState<string | null>(null)
  const [jobId, setJobId] = useState<string | null>(null)
  const [progress, setProgress] = useState<ToolboxProgress | null>(null)

  const onInput = useCallback((value: string) => {
    setFileName(null)
    setInput(value)
  }, [])
  useToolPendingInput(onInput)

  useEffect(() => {
    if (!jobId) return
    let unsub: (() => void) | undefined
    void listenToolboxProgress((event) => {
      if (event.jobId === jobId) setProgress(event)
    }).then((fn) => {
      unsub = fn
    })
    return () => {
      unsub?.()
    }
  }, [jobId])

  useEffect(() => {
    let cancelled = false
    async function run() {
      if (fileName || jobId) return
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
  }, [fileName, input, jobId])

  async function onBrowserFile(file: File | undefined) {
    if (!file) {
      setFileName(null)
      return
    }
    if (file.size > SMALL_FILE_MAX_BYTES) {
      setError(
        `File is larger than ${SMALL_FILE_MAX_BYTES} bytes. Use “Hash from disk” on the desktop app.`
      )
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

  async function onDiskFile() {
    if (!toolboxRustAvailable()) {
      setError("Large-file hashing needs the desktop app.")
      return
    }
    const { open } = await import("@tauri-apps/plugin-dialog")
    const path = await open({ multiple: false, title: "Hash file" })
    if (typeof path !== "string" || !path) return
    const id = crypto.randomUUID()
    setJobId(id)
    setProgress({ jobId: id, kind: "hash", bytesDone: 0, bytesTotal: 0 })
    setFileName(path)
    setInput("")
    setError(null)
    try {
      const report = await toolboxHashFile(path, id)
      setResult(formatToolboxHashReport(report))
    } catch (err) {
      const message = toErrorMessage(err)
      if (message !== "Cancelled") setError(message)
    } finally {
      setJobId(null)
      setProgress(null)
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
                onChange={(event) =>
                  void onBrowserFile(event.target.files?.[0])
                }
              />
            </label>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => void onDiskFile()}
              disabled={!isLocalDesktop() || jobId != null}
            >
              Hash from disk
            </Button>
            {fileName ? (
              <span className="pb-1 text-xs text-muted-foreground">
                Hashing: {fileName}
              </span>
            ) : null}
          </ParamPanel>
          {progress && jobId ? (
            <FileJobBar
              label="Hashing file"
              bytesDone={progress.bytesDone}
              bytesTotal={progress.bytesTotal}
              onCancel={() => {
                void toolboxCancelJob(jobId)
              }}
            />
          ) : null}
          <Warn>MD5 / SHA-1: checksum only, not for passwords.</Warn>
        </div>
      }
    />
  )
}
