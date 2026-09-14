"use client"

import { Suspense } from "react"
import { useTranslations } from "next-intl"
import { CodegAgentSettings } from "@/components/settings/codeg-agent-settings"

export default function SettingsCodegAgentPage() {
  const t = useTranslations("SettingsPages")

  return (
    <Suspense
      fallback={
        <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
          {t("codegAgentLoading")}
        </div>
      }
    >
      <CodegAgentSettings />
    </Suspense>
  )
}
