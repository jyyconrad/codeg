"use client"

import { Button } from "@/components/ui/button"
import { Progress } from "@/components/ui/progress"

export function FileJobBar({
  label,
  bytesDone,
  bytesTotal,
  onCancel,
}: {
  label: string
  bytesDone: number
  bytesTotal: number
  onCancel?: () => void
}) {
  const pct =
    bytesTotal > 0
      ? Math.min(100, Math.round((bytesDone / bytesTotal) * 100))
      : 0
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
        <span className="truncate">
          {label}
          {bytesTotal > 0 ? ` · ${pct}%` : ""}
        </span>
        {onCancel ? (
          <Button type="button" variant="outline" size="xs" onClick={onCancel}>
            Cancel
          </Button>
        ) : null}
      </div>
      <Progress value={pct} />
    </div>
  )
}
