import { EncodingError } from "./error"

export const RADIX_MIN = 2
export const RADIX_MAX = 36

export function convertRadix(
  value: string,
  fromRadix: number,
  toRadix: number
): string {
  assertRadix(fromRadix)
  assertRadix(toRadix)
  const trimmed = value.trim()
  if (trimmed === "") return ""

  let sign = 1n
  let body = trimmed
  if (body.startsWith("+")) {
    body = body.slice(1)
  } else if (body.startsWith("-")) {
    sign = -1n
    body = body.slice(1)
  }
  if (body === "") {
    throw new EncodingError("Invalid integer")
  }

  const magnitude = parseRadixInteger(body, fromRadix)
  const number = magnitude * sign
  if (number === 0n) return "0"
  return formatRadixInteger(number, toRadix)
}

export function parseRadixInteger(body: string, radix: number): bigint {
  assertRadix(radix)
  if (body === "") {
    throw new EncodingError("Invalid integer")
  }
  const base = BigInt(radix)
  let value = 0n
  for (const ch of body) {
    const digit = digitValue(ch)
    if (digit < 0 || digit >= radix) {
      throw new EncodingError(`Invalid digit '${ch}' for base ${radix}`)
    }
    value = value * base + BigInt(digit)
  }
  return value
}

export function formatRadixInteger(value: bigint, radix: number): string {
  assertRadix(radix)
  return value.toString(radix)
}

export function assertRadix(radix: number): void {
  if (!Number.isInteger(radix) || radix < RADIX_MIN || radix > RADIX_MAX) {
    throw new EncodingError("Radix must be an integer from 2 to 36")
  }
}

function digitValue(ch: string): number {
  const code = ch.charCodeAt(0)
  if (code >= 48 && code <= 57) return code - 48
  if (code >= 65 && code <= 90) return code - 55
  if (code >= 97 && code <= 122) return code - 87
  return -1
}
