import { afterEach, describe, expect, it, vi } from "vitest"
import {
  generateUuidBatch,
  generateUuidV4,
  generateUuidV7,
  isUuid,
} from "./uuid.core"

describe("uuid", () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it("generates RFC-like v4 strings without Math.random", () => {
    const spy = vi.spyOn(Math, "random")
    const id = generateUuidV4()
    expect(isUuid(id)).toBe(true)
    expect(id[14]).toBe("4")
    expect(id[19]).toMatch(/[89ab]/)
    expect(spy).not.toHaveBeenCalled()
  })

  it("encodes the millisecond timestamp in UUID v7", () => {
    const spy = vi.spyOn(Math, "random")
    const now = 1700000000000
    const rand = new Uint8Array(10).fill(0x11)
    const id = generateUuidV7(now, rand)
    expect(isUuid(id)).toBe(true)
    const hex = id.replace(/-/g, "")
    expect(Number.parseInt(hex.slice(0, 12), 16)).toBe(now)
    expect(hex[12]).toBe("7")
    expect(hex[16]).toMatch(/[89ab]/)
    expect(spy).not.toHaveBeenCalled()
  })

  it("batches unique ids", () => {
    const result = generateUuidBatch("v4", 5)
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(result.ids).toHaveLength(5)
    expect(new Set(result.ids).size).toBe(5)
    expect(result.output.split("\n")).toEqual(result.ids)
  })

  it("rejects counts outside 1–100", () => {
    expect(generateUuidBatch("v4", 0).ok).toBe(false)
    expect(generateUuidBatch("v7", 101).ok).toBe(false)
  })

  it("uses getRandomValues for v7", () => {
    const getRandomValues = vi.spyOn(globalThis.crypto, "getRandomValues")
    generateUuidV7(1)
    expect(getRandomValues).toHaveBeenCalled()
  })
})
