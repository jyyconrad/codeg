"use client"

import { Suspense, lazy, useMemo } from "react"
import { Loader2 } from "lucide-react"
import { useTranslations } from "next-intl"
import { TOOL_LOADERS } from "./tool-loaders"
import { useToolboxStore } from "./toolbox-store"

export function ToolboxWorkspace() {
  const t = useTranslations("Toolbox")
  const selectedToolId = useToolboxStore((s) => s.selectedToolId)

  const Tool = useMemo(() => {
    if (!selectedToolId) return null
    const loader = TOOL_LOADERS[selectedToolId]
    return lazy(loader)
  }, [selectedToolId])

  if (!Tool) {
    return (
      <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
        {t("emptyHint")}
      </div>
    )
  }

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <Suspense
        fallback={
          <div className="flex flex-1 items-center justify-center">
            <Loader2 className="size-5 animate-spin text-muted-foreground/60" />
          </div>
        }
      >
        <Tool />
      </Suspense>
    </div>
  )
}
