"use client"

import type { ReactNode } from "react"
import { useTranslations } from "next-intl"

const SELECT_CLASS =
  "h-8 rounded-full border border-border bg-input/30 px-2 text-sm text-foreground"

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
    <select
      aria-label={ariaLabel}
      className={SELECT_CLASS}
      value={value}
      onChange={(event) => onChange(event.target.value)}
    >
      {options.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  )
}
