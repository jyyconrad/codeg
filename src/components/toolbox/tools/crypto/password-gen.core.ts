import { CryptoToolError } from "./encoding"
import { randomInt } from "./random-int"

export const PASSWORD_SETS = {
  lower: "abcdefghijklmnopqrstuvwxyz",
  upper: "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
  digits: "0123456789",
  symbols: "!@#$%^&*()-_=+[]{};:,.<>?",
} as const

export type PasswordSetId = keyof typeof PASSWORD_SETS

const SIMILAR = new Set(["0", "O", "I", "l", "1"])

export interface PasswordGenOptions {
  length: number
  sets: readonly PasswordSetId[]
  excludeSimilar?: boolean
}

export function charsetFromOptions(options: PasswordGenOptions): string {
  const seen = new Set<string>()
  let out = ""
  for (const id of options.sets) {
    for (const ch of PASSWORD_SETS[id]) {
      if (options.excludeSimilar && SIMILAR.has(ch)) continue
      if (seen.has(ch)) continue
      seen.add(ch)
      out += ch
    }
  }
  return out
}

export function generatePassword(
  options: PasswordGenOptions,
  nextIndex: (max: number) => number = randomInt
): string {
  if (!Number.isInteger(options.length) || options.length < 1) {
    throw new CryptoToolError(
      "invalid-input",
      "Password length must be a positive integer."
    )
  }
  const charset = charsetFromOptions(options)
  if (charset.length === 0) {
    throw new CryptoToolError(
      "invalid-input",
      "Select at least one character set."
    )
  }
  let out = ""
  for (let i = 0; i < options.length; i += 1) {
    out += charset[nextIndex(charset.length)]
  }
  return out
}
