"use client"

import type { ReactNode } from "react"
import { useTranslations } from "next-intl"
import { ToolboxSelect } from "@/components/toolbox/toolbox-controls"

export function EncodingParams({ children }: { children: ReactNode }) {
  const t = useTranslations("Toolbox")
  return (
    <div className="flex flex-wrap items-center gap-2">
      <span className="text-xs font-medium text-muted-foreground">
        {t("params")}
      </span>
      {children}
    </div>
  )
}

export function EncodingSelect({
  value,
  onChange,
  options,
  "aria-label": ariaLabel,
}: {
  value: string
  onChange: (value: string) => void
  options: readonly { value: string; label: string }[]
  "aria-label": string
}) {
  return (
    <ToolboxSelect
      aria-label={ariaLabel}
      value={value}
      onChange={onChange}
      options={options}
    />
  )
}
