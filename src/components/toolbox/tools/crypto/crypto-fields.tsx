"use client"

import { type ReactNode } from "react"
import { ToolboxSelect } from "@/components/toolbox/toolbox-controls"
import { cn } from "@/lib/utils"

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
    <div className={cn("flex min-w-[9rem] flex-col gap-1", className)}>
      <span className="text-xs font-medium text-muted-foreground">{label}</span>
      {children}
    </div>
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
    <ToolboxSelect
      value={value}
      onChange={onChange}
      options={options}
      className={cn("w-full", className)}
    />
  )
}

export function Warn({ children }: { children: ReactNode }) {
  return (
    <p className="text-xs text-amber-700 dark:text-amber-400">{children}</p>
  )
}
