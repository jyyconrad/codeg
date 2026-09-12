"use client"

import { useState } from "react"
import { BookMarked } from "lucide-react"
import { useTranslations } from "next-intl"

import { WorkbenchPageTitle } from "@/components/workbench/workbench-page-title"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { isLocalDesktop } from "@/lib/platform"
import { WikiCapabilitiesView } from "./wiki-capabilities-view"
import { WikiJobsView } from "./wiki-jobs-view"
import { WikiSourcesView } from "./wiki-sources-view"
import { WikiWorkView } from "./wiki-work-view"

export type WikiViewId = "work" | "capabilities" | "sources" | "jobs"

export function WikiPageTitle() {
  const t = useTranslations("Wiki")
  return <WorkbenchPageTitle title={t("title")} />
}

function OpenInObsidianButton() {
  const t = useTranslations("Wiki")
  const desktop = isLocalDesktop()
  if (!desktop) return null
  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger asChild>
          <span>
            <Button type="button" variant="outline" size="sm" disabled>
              <BookMarked className="size-3.5" />
              {t("openInObsidian")}
            </Button>
          </span>
        </TooltipTrigger>
        <TooltipContent>{t("openInObsidianSoon")}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  )
}

export function WikiPage() {
  const t = useTranslations("Wiki")
  const [view, setView] = useState<WikiViewId>("work")

  return (
    <Tabs
      value={view}
      onValueChange={(value) => setView(value as WikiViewId)}
      className="flex h-full min-h-0 w-full flex-col gap-0"
    >
      <div className="flex shrink-0 items-center gap-2 border-b px-3 py-2">
        <TabsList>
          <TabsTrigger value="work">{t("views.work")}</TabsTrigger>
          <TabsTrigger value="capabilities">
            {t("views.capabilities")}
          </TabsTrigger>
          <TabsTrigger value="sources">{t("views.sources")}</TabsTrigger>
          <TabsTrigger value="jobs">{t("views.jobs")}</TabsTrigger>
        </TabsList>
        <div className="ml-auto">
          <OpenInObsidianButton />
        </div>
      </div>
      <TabsContent value="work" className="min-h-0 overflow-hidden">
        <WikiWorkView />
      </TabsContent>
      <TabsContent value="capabilities" className="min-h-0 overflow-hidden">
        <WikiCapabilitiesView />
      </TabsContent>
      <TabsContent value="sources" className="min-h-0 overflow-hidden">
        <WikiSourcesView />
      </TabsContent>
      <TabsContent value="jobs" className="min-h-0 overflow-hidden">
        <WikiJobsView />
      </TabsContent>
    </Tabs>
  )
}
