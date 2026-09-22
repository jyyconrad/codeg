function compactUnit(n: number, divisor: number, suffix: string): string {
  return `${(n / divisor).toFixed(1).replace(/\.0$/, "")}${suffix}`
}

export function formatTokenCount(n: number): string {
  if (n >= 1_000_000_000) return compactUnit(n, 1_000_000_000, "B")
  if (n >= 1_000_000) return compactUnit(n, 1_000_000, "M")
  if (n >= 1_000) return compactUnit(n, 1_000, "K")
  return n.toLocaleString()
}
