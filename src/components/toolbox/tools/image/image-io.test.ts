import { describe, expect, it } from "vitest"
import {
  IMAGE_LIMIT_MESSAGE,
  MAX_IMAGE_BYTES,
  MAX_IMAGE_EDGE,
  clampQuality,
  computeScaledSize,
  extForFormat,
  formatByteSize,
  guardImageFile,
  guardImageSize,
  mimeForFormat,
  withExtension,
} from "./image-io"

describe("image guards", () => {
  it("rejects files over 20MB", () => {
    expect(guardImageFile({ size: MAX_IMAGE_BYTES })).toBeNull()
    expect(guardImageFile({ size: MAX_IMAGE_BYTES + 1 })).toBe(
      IMAGE_LIMIT_MESSAGE
    )
  })

  it("rejects edges over 4096px", () => {
    expect(guardImageSize(MAX_IMAGE_EDGE, MAX_IMAGE_EDGE)).toBeNull()
    expect(guardImageSize(MAX_IMAGE_EDGE + 1, 10)).toBe(IMAGE_LIMIT_MESSAGE)
    expect(guardImageSize(10, MAX_IMAGE_EDGE + 1)).toBe(IMAGE_LIMIT_MESSAGE)
    expect(guardImageSize(0, 10)).toBe(IMAGE_LIMIT_MESSAGE)
  })
})

describe("compress sizing", () => {
  it("scales by max-width and keeps aspect", () => {
    expect(computeScaledSize(4000, 2000, 1000)).toEqual({
      width: 1000,
      height: 500,
    })
    expect(computeScaledSize(800, 600, 1920)).toEqual({
      width: 800,
      height: 600,
    })
  })

  it("maps export formats", () => {
    expect(mimeForFormat("jpeg")).toBe("image/jpeg")
    expect(mimeForFormat("webp")).toBe("image/webp")
    expect(extForFormat("png")).toBe("png")
    expect(withExtension("photo.PNG", "jpg")).toBe("photo.jpg")
    expect(clampQuality(2)).toBe(1)
    expect(clampQuality(-1)).toBe(0.01)
  })
})

describe("formatByteSize", () => {
  it("formats before/after byte counts", () => {
    expect(formatByteSize(0)).toBe("0 B")
    expect(formatByteSize(1023)).toBe("1023 B")
    expect(formatByteSize(1536)).toBe("1.5 KB")
    expect(formatByteSize(2 * 1024 * 1024)).toBe("2.00 MB")
  })
})
