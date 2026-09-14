import CryptoJS from "crypto-js"

export function bytesToWordArray(bytes: Uint8Array): CryptoJS.lib.WordArray {
  const words: number[] = []
  for (let i = 0; i < bytes.length; i += 4) {
    words.push(
      (((bytes[i] ?? 0) << 24) |
        ((bytes[i + 1] ?? 0) << 16) |
        ((bytes[i + 2] ?? 0) << 8) |
        (bytes[i + 3] ?? 0)) >>>
        0
    )
  }
  return CryptoJS.lib.WordArray.create(words, bytes.length)
}

export function wordArrayToBytes(
  wordArray: CryptoJS.lib.WordArray
): Uint8Array {
  const { words, sigBytes } = wordArray
  const out = new Uint8Array(sigBytes)
  for (let i = 0; i < sigBytes; i += 1) {
    out[i] = (words[i >>> 2]! >>> (24 - (i % 4) * 8)) & 0xff
  }
  return out
}
