/**
 * Wiki 设置的静态页面入口，挂载共享设置组件。
 * 设置加载、保存和返回 Wiki 的业务交互由 WikiSettings 组件负责。
 */
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
