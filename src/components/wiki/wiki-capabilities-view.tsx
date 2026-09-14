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
    />
  )
}
