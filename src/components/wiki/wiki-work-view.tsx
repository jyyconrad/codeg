"use client"

import { useTranslations } from "next-intl"

import { WikiNoteBrowser } from "./wiki-shared"

export function WikiWorkView() {
  const t = useTranslations("Wiki")
  return (
    <WikiNoteBrowser
      prefix="work"
      emptyTitle={t("work.emptyTitle")}
      emptyDescription={t("work.emptyDescription")}
    />
  )
}
