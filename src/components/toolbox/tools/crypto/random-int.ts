import { CryptoToolError } from "./encoding"

/**
 * Unbiased integer in `[0, maxExclusive)`. Uses rejection sampling so
 * `getRandomValues` bytes are not taken modulo a non-power-of-two.
 */
export function randomInt(
  maxExclusive: number,
  fill: (buffer: Uint8Array) => Uint8Array = (buffer) =>
    crypto.getRandomValues(buffer)
): number {
  if (
    !Number.isInteger(maxExclusive) ||
    maxExclusive < 1 ||
    maxExclusive > 256
  ) {
    throw new CryptoToolError(
      "invalid-input",
      "randomInt maxExclusive must be an integer in 1..256."
    )
  }
  if (maxExclusive === 1) return 0
  const limit = Math.floor(256 / maxExclusive) * maxExclusive
  const buf = new Uint8Array(1)
  for (;;) {
    fill(buf)
    const value = buf[0]!
    if (value < limit) return value % maxExclusive
  }
}
