"use client"

import { Button } from "@/components/ui/button"

export function ImageResultPreview({
  url,
  downloadLabel,
  onDownload,
  meta,
}: {
  url: string | null
  downloadLabel: string
  onDownload: () => void
  meta?: string
}) {
  if (!url) {
    return (
      <div className="flex min-h-[12rem] flex-1 items-center justify-center rounded-md border border-dashed text-sm text-muted-foreground" />
    )
  }

  return (
    <div className="flex min-h-[12rem] flex-1 flex-col gap-2">
      {meta ? <p className="text-xs text-muted-foreground">{meta}</p> : null}
      {/* Local blob preview; next/image cannot take object URLs. */}
      {/* eslint-disable-next-line @next/next/no-img-element */}
      <img
        src={url}
        alt=""
        className="max-h-80 w-fit max-w-full rounded-md border object-contain"
      />
      <Button type="button" variant="outline" size="sm" onClick={onDownload}>
        {downloadLabel}
      </Button>
    </div>
  )
}
