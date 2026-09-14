"use client"

import { Suspense } from "react"
import { useTranslations } from "next-intl"
import { WikiSettings } from "@/components/settings/wiki-settings"

export default function SettingsWikiPage() {
  const t = useTranslations("SettingsPages")

  return (
    <Suspense
      fallback={
        <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
          {t("wikiLoading")}
        </div>
      }
    >
      <WikiSettings />
    </Suspense>
  )
}
