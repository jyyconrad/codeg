"use client"

import { useCallback, useRef, useState } from "react"
import { useTranslations } from "next-intl"
import { QRCodeCanvas, QRCodeSVG } from "qrcode.react"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  TOOL_LABEL_CLASS,
  TOOL_SELECT_CLASS,
  canvasToBlob,
  triggerBlobDownload,
} from "@/components/toolbox/tools/image/image-io"
import {
  QR_ECC_LEVELS,
  isQrEccLevel,
  normalizeQrPayload,
  type QrEccLevel,
} from "./qr"

export default function QrGenerateTool() {
  const t = useTranslations("Toolbox")
  const [input, setInput] = useState("")
  const [level, setLevel] = useState<QrEccLevel>("M")
  const [size, setSize] = useState(256)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const svgRef = useRef<SVGSVGElement>(null)
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const payload = normalizeQrPayload(input)
  const ready = payload.trim().length > 0
  const pixelSize = Math.min(1024, Math.max(64, Math.round(size) || 256))

  async function downloadPng() {
    const canvas = canvasRef.current
    if (!canvas) return
    const blob = await canvasToBlob(canvas, "image/png")
    triggerBlobDownload(blob, "qr.png")
  }

  function downloadSvg() {
    const svg = svgRef.current
    if (!svg) return
    const xml = new XMLSerializer().serializeToString(svg)
    const blob = new Blob([xml], { type: "image/svg+xml;charset=utf-8" })
    triggerBlobDownload(blob, "qr.svg")
  }

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      inputLabel={t("input")}
      downloadFilename="qr.svg"
      result={ready ? payload : ""}
      example="https://example.com"
      params={
        <div className="flex flex-col gap-2">
          <p className="text-xs font-medium text-muted-foreground">
            {t("params")}
          </p>
          <div className="flex flex-wrap items-end gap-3">
            <label className={TOOL_LABEL_CLASS}>
              L / M / Q / H
              <select
                className={TOOL_SELECT_CLASS}
                value={level}
                onChange={(event) => {
                  const next = event.target.value
                  if (isQrEccLevel(next)) setLevel(next)
                }}
              >
                {QR_ECC_LEVELS.map((item) => (
                  <option key={item} value={item}>
                    {item}
                  </option>
                ))}
              </select>
            </label>
            <label className={TOOL_LABEL_CLASS}>
              px
              <Input
                type="number"
                min={64}
                max={1024}
                className="h-8 w-24"
                value={pixelSize}
                onChange={(event) => setSize(Number(event.target.value) || 256)}
              />
            </label>
          </div>
        </div>
      }
      resultSlot={
        ready ? (
          <div className="flex min-h-[12rem] flex-1 flex-col gap-3">
            <div className="w-fit rounded-md bg-white p-2">
              <QRCodeSVG
                ref={svgRef}
                value={payload}
                size={pixelSize}
                level={level}
                marginSize={2}
                title={t("tools.qr-generate.title")}
              />
            </div>
            <div className="hidden">
              <QRCodeCanvas
                ref={canvasRef}
                value={payload}
                size={pixelSize}
                level={level}
                marginSize={2}
              />
            </div>
            <div className="flex flex-wrap gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => void downloadPng()}
              >
                {t("download")} PNG
              </Button>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={downloadSvg}
              >
                {t("download")} SVG
              </Button>
            </div>
          </div>
        ) : undefined
      }
    />
  )
}
