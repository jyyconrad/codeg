"use client"

import { useCallback, useEffect, useMemo, useState } from "react"
import { useTranslations } from "next-intl"
import { FileJobBar } from "@/components/toolbox/file-job-bar"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { toErrorMessage } from "@/lib/app-error"
import { isLocalDesktop } from "@/lib/platform"
import {
  listenToolboxProgress,
  toolboxCancelJob,
  toolboxCipherFile,
  toolboxRustAvailable,
  type ToolboxProgress,
} from "@/lib/toolbox-api"
import { ParamField, ParamPanel, ParamSelect, Warn } from "./crypto-fields"
import { type ByteEncoding, errorMessage } from "./encoding"
import type { PaddingMode } from "./padding"
import {
  type CipherDirection,
  type CipherMode,
  type SymmetricAlgorithm,
  generateIv,
  runSymmetric,
} from "./symmetric"

const ALGORITHMS: { value: SymmetricAlgorithm; label: string }[] = [
  { value: "aes-128", label: "AES-128" },
  { value: "aes-192", label: "AES-192" },
  { value: "aes-256", label: "AES-256" },
  { value: "sm4", label: "SM4" },
]

const MODES: { value: CipherMode; label: string }[] = [
  { value: "cbc", label: "CBC" },
  { value: "gcm", label: "GCM" },
  { value: "ecb", label: "ECB" },
]

const PADDINGS: { value: PaddingMode; label: string }[] = [
  { value: "pkcs7", label: "PKCS7" },
  { value: "zero", label: "ZeroPadding" },
  { value: "none", label: "NoPadding" },
]

const ENCODINGS: { value: ByteEncoding; label: string }[] = [
  { value: "utf8", label: "UTF-8" },
  { value: "hex", label: "Hex" },
  { value: "base64", label: "Base64" },
]

const BYTE_ENCODINGS: { value: ByteEncoding; label: string }[] = [
  { value: "hex", label: "Hex" },
  { value: "base64", label: "Base64" },
]

export default function SymmetricCipherTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const [algorithm, setAlgorithm] = useState<SymmetricAlgorithm>("aes-128")
  const [mode, setMode] = useState<CipherMode>("cbc")
  const [padding, setPadding] = useState<PaddingMode>("pkcs7")
  const [direction, setDirection] = useState<CipherDirection>("encrypt")
  const [key, setKey] = useState("")
  const [keyEncoding, setKeyEncoding] = useState<ByteEncoding>("utf8")
  const [iv, setIv] = useState("")
  const [ivEncoding, setIvEncoding] = useState<ByteEncoding>("utf8")
  const [inputEncoding, setInputEncoding] = useState<ByteEncoding>("utf8")
  const [outputEncoding, setOutputEncoding] = useState<ByteEncoding>("base64")
  const [prependIv, setPrependIv] = useState(false)
  const [passphrase, setPassphrase] = useState("")
  const [result, setResult] = useState("")
  const [error, setError] = useState<string | null>(null)
  const [jobId, setJobId] = useState<string | null>(null)
  const [progress, setProgress] = useState<ToolboxProgress | null>(null)
  const [fileStatus, setFileStatus] = useState<string | null>(null)

  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const inputEncodings = useMemo(() => {
    if (direction === "encrypt") return ENCODINGS
    return BYTE_ENCODINGS
  }, [direction])

  const outputEncodings = useMemo(() => {
    if (direction === "decrypt") return ENCODINGS
    return BYTE_ENCODINGS
  }, [direction])

  const effectiveInputEncoding: ByteEncoding =
    direction === "decrypt" && inputEncoding === "utf8"
      ? "base64"
      : inputEncoding
  const effectiveOutputEncoding: ByteEncoding =
    direction === "encrypt" && outputEncoding === "utf8"
      ? "base64"
      : outputEncoding

  useEffect(() => {
    let cancelled = false
    async function run() {
      if (!input.trim() || !key.trim()) {
        setResult("")
        setError(null)
        return
      }
      try {
        const out = await runSymmetric({
          algorithm,
          mode,
          padding,
          direction,
          key,
          keyEncoding,
          iv,
          ivEncoding,
          input,
          inputEncoding: effectiveInputEncoding,
          outputEncoding: effectiveOutputEncoding,
          prependIv,
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
  }, [
    algorithm,
    direction,
    input,
    effectiveInputEncoding,
    iv,
    ivEncoding,
    key,
    keyEncoding,
    mode,
    effectiveOutputEncoding,
    padding,
    prependIv,
  ])

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

  async function runFileJob() {
    if (!toolboxRustAvailable()) {
      setError("File encryption needs the desktop app.")
      return
    }
    const { open, save } = await import("@tauri-apps/plugin-dialog")
    const srcPath = await open({
      multiple: false,
      title: direction === "encrypt" ? "Plaintext file" : "Ciphertext file",
    })
    if (typeof srcPath !== "string" || !srcPath) return
    const destPath = await save({
      defaultPath:
        direction === "encrypt"
          ? `${srcPath}.enc`
          : srcPath.replace(/\.enc$/i, ""),
      title: "Save output",
    })
    if (typeof destPath !== "string" || !destPath) return
    const id = crypto.randomUUID()
    setJobId(id)
    setProgress({ jobId: id, kind: "cipher", bytesDone: 0, bytesTotal: 0 })
    setError(null)
    setFileStatus(`${srcPath} → ${destPath}`)
    try {
      await toolboxCipherFile({
        srcPath,
        destPath,
        algorithm,
        mode,
        padding,
        direction,
        key,
        keyEncoding,
        iv,
        ivEncoding,
        prependIv,
        passphrase: passphrase.trim() ? passphrase : null,
        pbkdf2Iterations: passphrase.trim() ? 100_000 : null,
        jobId: id,
      })
      setFileStatus(`Wrote ${destPath}`)
    } catch (err) {
      const message = toErrorMessage(err)
      if (message !== "Cancelled") setError(message)
      setFileStatus(null)
    } finally {
      setJobId(null)
      setProgress(null)
    }
  }

  function onExample() {
    setDirection("encrypt")
    setAlgorithm("aes-128")
    setMode("cbc")
    setPadding("pkcs7")
    setKeyEncoding("utf8")
    setIvEncoding("utf8")
    setInputEncoding("utf8")
    setOutputEncoding("base64")
    setKey("1234567890123456")
    setIv("1234567890123456")
    setInput("Hello, Codeg")
  }

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      inputLabel={direction === "encrypt" ? "Plaintext" : "Ciphertext"}
      result={result}
      error={error}
      cryptoFooter
      onExample={onExample}
      params={
        <div className="flex flex-col gap-3">
          <ParamPanel title={t("params")}>
            <ParamField label="Direction">
              <ParamSelect
                value={direction}
                onChange={(value) => setDirection(value as CipherDirection)}
                options={[
                  { value: "encrypt", label: "Encrypt" },
                  { value: "decrypt", label: "Decrypt" },
                ]}
              />
            </ParamField>
            <ParamField label="Algorithm">
              <ParamSelect
                value={algorithm}
                onChange={(value) => setAlgorithm(value as SymmetricAlgorithm)}
                options={ALGORITHMS}
              />
            </ParamField>
            <ParamField label="Mode">
              <ParamSelect
                value={mode}
                onChange={(value) => setMode(value as CipherMode)}
                options={MODES}
              />
            </ParamField>
            <ParamField label="Padding">
              <ParamSelect
                value={padding}
                onChange={(value) => setPadding(value as PaddingMode)}
                options={PADDINGS}
                className={mode === "gcm" ? "opacity-60" : undefined}
              />
            </ParamField>
            <ParamField label="Key encoding">
              <ParamSelect
                value={keyEncoding}
                onChange={(value) => setKeyEncoding(value as ByteEncoding)}
                options={ENCODINGS}
              />
            </ParamField>
            {mode !== "ecb" ? (
              <ParamField label="IV encoding">
                <ParamSelect
                  value={ivEncoding}
                  onChange={(value) => setIvEncoding(value as ByteEncoding)}
                  options={ENCODINGS}
                />
              </ParamField>
            ) : null}
            <ParamField label="Input encoding">
              <ParamSelect
                value={effectiveInputEncoding}
                onChange={(value) => setInputEncoding(value as ByteEncoding)}
                options={inputEncodings}
              />
            </ParamField>
            <ParamField label="Output encoding">
              <ParamSelect
                value={effectiveOutputEncoding}
                onChange={(value) => setOutputEncoding(value as ByteEncoding)}
                options={outputEncodings}
              />
            </ParamField>
            {mode !== "ecb" ? (
              <label className="flex items-center gap-2 pb-1 text-xs text-muted-foreground">
                <input
                  type="checkbox"
                  checked={prependIv}
                  onChange={(event) => setPrependIv(event.target.checked)}
                />
                Prepend IV to ciphertext
              </label>
            ) : null}
          </ParamPanel>
          <div className="flex flex-wrap items-end gap-3">
            <ParamField label="Key" className="min-w-[16rem] flex-1">
              <Input
                value={key}
                onChange={(event) => setKey(event.target.value)}
                className="font-mono"
                autoComplete="off"
              />
            </ParamField>
            {mode !== "ecb" ? (
              <ParamField label="IV / nonce" className="min-w-[16rem] flex-1">
                <Input
                  value={iv}
                  onChange={(event) => setIv(event.target.value)}
                  className="font-mono"
                  autoComplete="off"
                />
              </ParamField>
            ) : null}
            {mode !== "ecb" ? (
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => {
                  const encoding = ivEncoding === "utf8" ? "hex" : ivEncoding
                  if (ivEncoding === "utf8") setIvEncoding("hex")
                  setIv(generateIv(mode, encoding))
                }}
              >
                Generate random IV
              </Button>
            ) : null}
          </div>
          {mode === "ecb" ? (
            <Warn>ECB is legacy only — do not use for new systems.</Warn>
          ) : null}
          {mode === "gcm" ? (
            <p className="text-xs text-muted-foreground">
              GCM does not use padding. Nonce defaults to 12 bytes. Ciphertext
              includes a 16-byte auth tag. File GCM is capped at 64 MiB.
            </p>
          ) : null}
          <ParamPanel title="File (desktop)">
            <ParamField
              label="Optional passphrase (PBKDF2)"
              className="min-w-[16rem] flex-1"
            >
              <Input
                type="password"
                value={passphrase}
                onChange={(event) => setPassphrase(event.target.value)}
                autoComplete="off"
              />
            </ParamField>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={!isLocalDesktop() || jobId != null}
              onClick={() => void runFileJob()}
            >
              {direction === "encrypt" ? "Encrypt file…" : "Decrypt file…"}
            </Button>
          </ParamPanel>
          {progress && jobId ? (
            <FileJobBar
              label="Processing file"
              bytesDone={progress.bytesDone}
              bytesTotal={progress.bytesTotal}
              onCancel={() => {
                void toolboxCancelJob(jobId)
              }}
            />
          ) : null}
          {fileStatus ? (
            <p className="text-xs text-muted-foreground">{fileStatus}</p>
          ) : null}
        </div>
      }
    />
  )
}
