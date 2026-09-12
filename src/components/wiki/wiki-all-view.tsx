"use client"

import { useState } from "react"
import { useTranslations } from "next-intl"

import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { WikiNoteBrowser } from "./wiki-shared"

export function WikiAllView() {
  const t = useTranslations("Wiki")
  const [includeRaw, setIncludeRaw] = useState(false)

  return (
    <WikiNoteBrowser
      prefix=""
      includeRaw={includeRaw}
      emptyTitle={t("all.emptyTitle")}
      emptyDescription={t("all.emptyDescription")}
      extras={
        <Label className="flex items-center gap-2 text-sm font-normal text-muted-foreground">
          <Switch checked={includeRaw} onCheckedChange={setIncludeRaw} />
          {t("showRaw")}
        </Label>
      }
    />
  )
}
