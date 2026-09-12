"use client"

import { useTranslations } from "next-intl"

import { WikiNoteBrowser } from "./wiki-shared"

export function WikiCapabilitiesView() {
  const t = useTranslations("Wiki")
  return (
    <WikiNoteBrowser
      prefix="capabilities"
      emptyTitle={t("capabilities.emptyTitle")}
      emptyDescription={t("capabilities.emptyDescription")}
      extras={
        <ul className="list-disc space-y-1 pl-5 text-sm leading-6 text-muted-foreground">
          <li>{t("capabilities.evidenceReference")}</li>
          <li>{t("capabilities.evidenceApplication")}</li>
          <li>{t("capabilities.evidenceResult")}</li>
          <li>{t("capabilities.evidenceReflection")}</li>
        </ul>
      }
    />
  )
}
