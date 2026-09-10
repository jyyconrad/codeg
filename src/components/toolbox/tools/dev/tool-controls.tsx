import type { ReactNode } from "react"

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
    <label className="flex items-center gap-2">
      {label}
      <select
        className="h-8 rounded-full border border-border bg-input/30 px-2 text-foreground"
        value={value}
        onChange={(event) => onChange(event.target.value)}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </label>
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
  return (
    <label className="flex items-center gap-2">
      <input
        type="checkbox"
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
      />
      {label}
    </label>
  )
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
    <label className="flex items-center gap-2">
      {label}
      <input
        type="number"
        min={min}
        max={max}
        className="h-8 w-20 rounded-full border border-border bg-input/30 px-2 text-foreground"
        value={Number.isFinite(value) ? value : ""}
        onChange={(event) => onChange(Number(event.target.value))}
      />
    </label>
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
    <label className="flex min-w-[12rem] flex-1 items-center gap-2">
      {label}
      <input
        className={`h-8 min-w-0 flex-1 rounded-full border border-border bg-input/30 px-2 text-foreground ${
          mono ? "font-mono" : ""
        }`}
        value={value}
        placeholder={placeholder}
        onChange={(event) => onChange(event.target.value)}
      />
    </label>
  )
}
