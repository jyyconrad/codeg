/** FIPS 202 SHA3-256. crypto-js SHA3 uses Keccak padding (0x01), not SHA-3 (0x06). */

const RC = [
  0x0000000000000001n,
  0x0000000000008082n,
  0x800000000000808an,
  0x8000000080008000n,
  0x000000000000808bn,
  0x0000000080000001n,
  0x8000000080008081n,
  0x8000000000008009n,
  0x000000000000008an,
  0x0000000000000088n,
  0x0000000080008009n,
  0x000000008000000an,
  0x000000008000808bn,
  0x800000000000008bn,
  0x8000000000008089n,
  0x8000000000008003n,
  0x8000000000008002n,
  0x8000000000000080n,
  0x000000000000800an,
  0x800000008000000an,
  0x8000000080008081n,
  0x8000000000008080n,
  0x0000000080000001n,
  0x8000000080008008n,
]

const ROT = [
  0, 1, 62, 28, 27, 36, 44, 6, 55, 20, 3, 10, 43, 25, 39, 41, 45, 15, 21, 8, 18,
  2, 61, 56, 14,
]

const MASK = 0xffffffffffffffffn

function rotl(x: bigint, n: number): bigint {
  const s = BigInt(n)
  return ((x << s) | (x >> (64n - s))) & MASK
}

function keccakF(a: bigint[]): void {
  for (let round = 0; round < 24; round += 1) {
    const c: bigint[] = []
    for (let x = 0; x < 5; x += 1) {
      c[x] = a[x]! ^ a[x + 5]! ^ a[x + 10]! ^ a[x + 15]! ^ a[x + 20]!
    }
    const d: bigint[] = []
    for (let x = 0; x < 5; x += 1) {
      d[x] = c[(x + 4) % 5]! ^ rotl(c[(x + 1) % 5]!, 1)
    }
    for (let i = 0; i < 25; i += 1) a[i] = (a[i]! ^ d[i % 5]!) & MASK

    const b = new Array<bigint>(25).fill(0n)
    for (let x = 0; x < 5; x += 1) {
      for (let y = 0; y < 5; y += 1) {
        const src = x + 5 * y
        const dst = y + 5 * ((2 * x + 3 * y) % 5)
        b[dst] = rotl(a[src]!, ROT[src]!)
      }
    }
    for (let x = 0; x < 5; x += 1) {
      for (let y = 0; y < 5; y += 1) {
        const i = x + 5 * y
        a[i] =
          (b[i]! ^ (~b[((x + 1) % 5) + 5 * y]! & b[((x + 2) % 5) + 5 * y]!)) &
          MASK
      }
    }
    a[0] = (a[0]! ^ RC[round]!) & MASK
  }
}

function loadLanes(state: Uint8Array): bigint[] {
  const lanes: bigint[] = []
  const view = new DataView(state.buffer, state.byteOffset, state.byteLength)
  for (let i = 0; i < 25; i += 1) {
    lanes.push(view.getBigUint64(i * 8, true))
  }
  return lanes
}

function storeLanes(lanes: bigint[], state: Uint8Array): void {
  const view = new DataView(state.buffer, state.byteOffset, state.byteLength)
  for (let i = 0; i < 25; i += 1) {
    view.setBigUint64(i * 8, lanes[i]!, true)
  }
}

export function sha3_256(message: Uint8Array): Uint8Array {
  const rate = 136
  const state = new Uint8Array(200)
  let offset = 0
  for (let i = 0; i < message.length; i += 1) {
    state[offset] ^= message[i]!
    offset += 1
    if (offset === rate) {
      const lanes = loadLanes(state)
      keccakF(lanes)
      storeLanes(lanes, state)
      offset = 0
    }
  }
  state[offset] ^= 0x06
  state[rate - 1] ^= 0x80
  const lanes = loadLanes(state)
  keccakF(lanes)
  storeLanes(lanes, state)
  return state.slice(0, 32)
}
