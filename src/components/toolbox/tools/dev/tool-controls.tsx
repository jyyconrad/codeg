"use client"

import type { ReactNode } from "react"
import {
  ToolboxCheckbox,
  ToolboxNumber,
  ToolboxSelect,
  ToolboxText,
} from "@/components/toolbox/toolbox-controls"

export function ToolParams({ children }: { children: ReactNode }) {
  return (
    <div className="flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
      {children}
    </div>
  )
}

export function ToolSelect({
  label,
  value,
  onChange,
  options,
}: {
  label: string
  value: string
  onChange: (value: string) => void
  options: readonly { value: string; label: string }[]
}) {
  return (
    <ToolboxSelect
      label={label}
      value={value}
      onChange={onChange}
      options={options}
    />
  )
}

export function ToolCheckbox({
  label,
  checked,
  onChange,
}: {
  label: string
  checked: boolean
  onChange: (checked: boolean) => void
}) {
  return <ToolboxCheckbox label={label} checked={checked} onChange={onChange} />
}

export function ToolNumber({
  label,
  value,
  min,
  max,
  onChange,
}: {
  label: string
  value: number
  min?: number
  max?: number
  onChange: (value: number) => void
}) {
  return (
    <ToolboxNumber
      label={label}
      value={value}
      min={min}
      max={max}
      onChange={onChange}
    />
  )
}

export function ToolText({
  label,
  value,
  onChange,
  placeholder,
  mono,
}: {
  label: string
  value: string
  onChange: (value: string) => void
  placeholder?: string
  mono?: boolean
}) {
  return (
    <ToolboxText
      label={label}
      value={value}
      onChange={onChange}
      placeholder={placeholder}
      mono={mono}
    />
  )
}
