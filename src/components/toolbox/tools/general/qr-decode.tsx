"use client"

import { useCallback, useEffect, useState } from "react"
import { useTranslations } from "next-intl"
import jsQR from "jsqr"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { useLoadedImage } from "@/components/toolbox/tools/image/use-loaded-image"
import { formatQrDecodeResult } from "./qr"

export default function QrDecodeTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const [file, setFile] = useState<File | null>(null)
  const [payload, setPayload] = useState("")
  const [decodeError, setDecodeError] = useState<string | null>(null)
  const onInput = useCallback((value: string) => {
    setInput(value)
    if (!value) setFile(null)
  }, [])
  useToolPendingInput(onInput)

  const { image, error: loadError } = useLoadedImage(file)

  useEffect(() => {
    if (!image) return
    let cancelled = false
    const timer = window.setTimeout(() => {
      if (cancelled) return
      const canvas = document.createElement("canvas")
      canvas.width = image.naturalWidth
      canvas.height = image.naturalHeight
      const ctx = canvas.getContext("2d")
      if (!ctx) {
        setPayload("")
        setDecodeError("Export failed.")
        return
      }
      ctx.drawImage(image, 0, 0)
      const pixels = ctx.getImageData(0, 0, canvas.width, canvas.height)
      const found = jsQR(pixels.data, pixels.width, pixels.height)
      if (!found) {
        setPayload("")
        setDecodeError("No QR code found.")
        return
      }
      setPayload(formatQrDecodeResult(found.data))
      setDecodeError(null)
    }, 0)
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [image])

  const shownPayload = image ? payload : ""
  const shownDecodeError = image ? decodeError : null

  return (
    <ToolPageShell
      input={input}
      onInputChange={onInput}
      inputLabel={t("input")}
      downloadFilename="qr-payload.txt"
      result={shownPayload}
      error={loadError ?? shownDecodeError}
      params={
        <p className="text-xs text-muted-foreground">
          {t("tools.qr-decode.description")}
        </p>
      }
      inputSlot={
        <Input
          key={input || "empty"}
          type="file"
          accept="image/*"
          onChange={(event) => {
            const next = event.target.files?.[0] ?? null
            setFile(next)
            setInput(next?.name ?? "")
          }}
        />
      }
      resultSlot={
        <Textarea
          readOnly
          value={shownPayload}
          className="min-h-[12rem] flex-1 font-mono text-sm"
        />
      }
    />
  )
}
