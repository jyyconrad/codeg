"use client"

import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { cn } from "@/lib/utils"

export function ToolboxSelect({
  label,
  value,
  onChange,
  options,
  className,
  "aria-label": ariaLabel,
}: {
  label?: string
  value: string
  onChange: (value: string) => void
  options: readonly { value: string; label: string }[]
  className?: string
  "aria-label"?: string
}) {
  const select = (
    <Select value={value || undefined} onValueChange={onChange}>
      <SelectTrigger
        size="sm"
        className={cn("min-w-[7.5rem]", className)}
        aria-label={ariaLabel ?? label}
      >
        <SelectValue />
      </SelectTrigger>
      <SelectContent position="popper" align="start">
        {options.map((option) =>
          option.value === "" ? null : (
            <SelectItem key={option.value} value={option.value}>
              {option.label}
            </SelectItem>
          )
        )}
      </SelectContent>
    </Select>
  )

  if (!label) return select

  return (
    <div className="flex items-center gap-2 text-xs text-muted-foreground">
      <span className="shrink-0">{label}</span>
      {select}
    </div>
  )
}

export function ToolboxCheckbox({
  label,
  checked,
  onChange,
}: {
  label: string
  checked: boolean
  onChange: (checked: boolean) => void
}) {
  return (
    <Label className="flex items-center gap-2 text-xs font-normal text-muted-foreground">
      <Checkbox
        checked={checked}
        onCheckedChange={(value) => onChange(value === true)}
      />
      {label}
    </Label>
  )
}

export function ToolboxNumber({
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
    <Label className="flex items-center gap-2 text-xs font-normal text-muted-foreground">
      {label}
      <Input
        type="number"
        min={min}
        max={max}
        className="h-8 w-20"
        value={Number.isFinite(value) ? value : ""}
        onChange={(event) => onChange(Number(event.target.value))}
      />
    </Label>
  )
}

export function ToolboxText({
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
    <Label className="flex min-w-[12rem] flex-1 items-center gap-2 text-xs font-normal text-muted-foreground">
      {label}
      <Input
        className={cn("h-8 min-w-0 flex-1", mono && "font-mono")}
        value={value}
        placeholder={placeholder}
        onChange={(event) => onChange(event.target.value)}
      />
    </Label>
  )
}
