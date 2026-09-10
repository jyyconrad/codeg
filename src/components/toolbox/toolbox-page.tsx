"use client"

import { useTranslations } from "next-intl"
import { WorkbenchPageTitle } from "@/components/workbench/workbench-page-title"
import { ToolboxView } from "./toolbox-view"
import { useToolboxStore } from "./toolbox-store"

export function ToolboxPage() {
  return (
    <div className="h-full min-h-0 w-full">
      <ToolboxView />
    </div>
  )
}

export function ToolboxPageTitle() {
  const t = useTranslations("Toolbox")
  const selectedToolId = useToolboxStore((s) => s.selectedToolId)
  const current = selectedToolId ? t(`tools.${selectedToolId}.title`) : null
  return <WorkbenchPageTitle title={t("title")} current={current} />
}
