export interface PasswordStrength {
  length: number
  hasLower: boolean
  hasUpper: boolean
  hasDigit: boolean
  hasSymbol: boolean
  charsetSize: number
  entropyBits: number
  classes: number
  label: "very weak" | "weak" | "fair" | "strong" | "very strong"
}

const SYMBOL = /[^A-Za-z0-9]/

export function estimatePasswordStrength(password: string): PasswordStrength {
  const hasLower = /[a-z]/.test(password)
  const hasUpper = /[A-Z]/.test(password)
  const hasDigit = /\d/.test(password)
  const hasSymbol = SYMBOL.test(password)
  let charsetSize = 0
  if (hasLower) charsetSize += 26
  if (hasUpper) charsetSize += 26
  if (hasDigit) charsetSize += 10
  if (hasSymbol) charsetSize += 33
  const other = new Set<string>()
  for (const ch of password) {
    if (!/[A-Za-z0-9]/.test(ch) && !SYMBOL.test(ch)) other.add(ch)
  }
  charsetSize += other.size
  const classes =
    Number(hasLower) + Number(hasUpper) + Number(hasDigit) + Number(hasSymbol)
  const entropyBits =
    password.length === 0 || charsetSize === 0
      ? 0
      : password.length * Math.log2(charsetSize)
  let label: PasswordStrength["label"] = "very weak"
  if (entropyBits >= 80) label = "very strong"
  else if (entropyBits >= 60) label = "strong"
  else if (entropyBits >= 36) label = "fair"
  else if (entropyBits >= 28) label = "weak"
  return {
    length: password.length,
    hasLower,
    hasUpper,
    hasDigit,
    hasSymbol,
    charsetSize,
    entropyBits,
    classes,
    label,
  }
}

export function formatPasswordStrength(s: PasswordStrength): string {
  const flags = [
    s.hasLower ? "lowercase" : null,
    s.hasUpper ? "uppercase" : null,
    s.hasDigit ? "digits" : null,
    s.hasSymbol ? "symbols" : null,
  ].filter(Boolean)
  return [
    `Length: ${s.length}`,
    `Classes: ${s.classes} (${flags.join(", ") || "none"})`,
    `Estimated charset: ${s.charsetSize}`,
    `Estimated entropy: ${s.entropyBits.toFixed(1)} bits`,
    `Strength: ${s.label}`,
  ].join("\n")
}
