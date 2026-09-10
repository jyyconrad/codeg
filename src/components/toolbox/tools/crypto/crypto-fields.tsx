"use client"

import { type ChangeEvent, type ReactNode } from "react"
import { cn } from "@/lib/utils"

export const paramControlClass =
  "h-8 rounded-full border border-border bg-input/30 px-2 text-sm text-foreground"

export function ParamPanel({
  title,
  children,
}: {
  title: string
  children: ReactNode
}) {
  return (
    <div className="flex flex-col gap-2">
      <span className="text-xs font-medium text-muted-foreground">{title}</span>
      <div className="flex flex-wrap items-end gap-3">{children}</div>
    </div>
  )
}

export function ParamField({
  label,
  className,
  children,
}: {
  label: string
  className?: string
  children: ReactNode
}) {
  return (
    <label className={cn("flex min-w-[9rem] flex-col gap-1", className)}>
      <span className="text-xs font-medium text-muted-foreground">{label}</span>
      {children}
    </label>
  )
}

export function ParamSelect({
  value,
  onChange,
  options,
  className,
}: {
  value: string
  onChange: (value: string) => void
  options: readonly { value: string; label: string }[]
  className?: string
}) {
  return (
    <select
      className={cn(paramControlClass, className)}
      value={value}
      onChange={(event: ChangeEvent<HTMLSelectElement>) =>
        onChange(event.target.value)
      }
    >
      {options.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  )
}

export function Warn({ children }: { children: ReactNode }) {
  return (
    <p className="text-xs text-amber-700 dark:text-amber-400">{children}</p>
  )
}
