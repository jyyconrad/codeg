"use client"

import {
  Suspense,
  lazy,
  type ComponentType,
  type LazyExoticComponent,
} from "react"
import { Loader2 } from "lucide-react"
import { useTranslations } from "next-intl"
import { TOOL_LOADERS } from "./tool-loaders"
import { useToolboxStore } from "./toolbox-store"
import type { ToolboxToolId } from "./types"

const LAZY_TOOLS = Object.fromEntries(
  Object.entries(TOOL_LOADERS).map(([id, loader]) => [id, lazy(loader)])
) as Record<ToolboxToolId, LazyExoticComponent<ComponentType>>

export function ToolboxWorkspace() {
  const t = useTranslations("Toolbox")
  const selectedToolId = useToolboxStore((s) => s.selectedToolId)
  const Tool = selectedToolId ? LAZY_TOOLS[selectedToolId] : null

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
