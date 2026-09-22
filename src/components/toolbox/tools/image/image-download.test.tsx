import { cleanup, render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { NextIntlClientProvider } from "next-intl"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import enMessages from "@/i18n/messages/en.json"
import { ToolPageShell } from "../../tool-page-shell"
import { useToolboxStore } from "../../toolbox-store"
import ImageCompressTool from "./image-compress"
import ImageCropTool from "./image-crop"
import ImageStitchTool from "./image-stitch"
import ImageWatermarkTool from "./image-watermark"
import QrGenerateTool from "../general/qr-generate"

// jsdom has no image decoder or canvas encoder. Keep the components, drawing
// flow, object URL lifecycle and download handlers real; stub only these APIs.
vi.mock("./image-io", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./image-io")>()
  return {
    ...actual,
    loadImageFromUrl: async () => {
      const image = new Image()
      Object.defineProperties(image, {
        naturalWidth: { value: 1 },
        naturalHeight: { value: 1 },
      })
      return image
    },
  }
})

const pngBytes = Uint8Array.from(
  atob(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aT1sAAAAASUVORK5CYII="
  ),
  (char) => char.charCodeAt(0)
)

function readBlob(blob: Blob): Promise<ArrayBuffer> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(reader.result as ArrayBuffer)
    reader.onerror = () => reject(reader.error)
    reader.readAsArrayBuffer(blob)
  })
}

describe("toolbox image downloads", () => {
  const downloads: { filename: string; blob: Blob }[] = []

  beforeEach(() => {
    downloads.length = 0
    useToolboxStore.setState({ selectedToolId: null, pendingInput: null })
    const urls = new Map<string, Blob>()
    let urlId = 0
    vi.stubGlobal(
      "URL",
      Object.assign(class extends URL {}, {
        createObjectURL: (blob: Blob) => {
          const url = `blob:test-${++urlId}`
          urls.set(url, blob)
          return url
        },
        revokeObjectURL: (url: string) => urls.delete(url),
      })
    )
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (
      this: HTMLAnchorElement
    ) {
      const blob = urls.get(this.href)
      if (blob) downloads.push({ filename: this.download, blob })
    })
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({
      drawImage: vi.fn(),
      fillRect: vi.fn(),
      fillText: vi.fn(),
      save: vi.fn(),
      restore: vi.fn(),
      translate: vi.fn(),
      rotate: vi.fn(),
      scale: vi.fn(),
      measureText: () => ({ width: 32 }),
    } as unknown as CanvasRenderingContext2D)
    vi.spyOn(HTMLCanvasElement.prototype, "toBlob").mockImplementation(
      (callback, type) => callback(new Blob([pngBytes], { type }))
    )
  })

  afterEach(() => {
    cleanup()
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it.each([
    ["watermark", ImageWatermarkTool, "20260824095352_489_180.png"],
    ["compress", ImageCompressTool, "20260824095352_489_180.png"],
    ["crop", ImageCropTool, "20260824095352_489_180.png"],
    ["stitch", ImageStitchTool, "stitch.png"],
  ] as const)(
    "%s downloads the generated image from both download buttons",
    async (name, Tool, filename) => {
      const user = userEvent.setup()
      const { container } = render(
        <NextIntlClientProvider locale="en" messages={enMessages}>
          <Tool />
        </NextIntlClientProvider>
      )
      expect(screen.getByRole("button", { name: "Download" })).toBeDisabled()
      if (name === "compress") {
        await user.click(screen.getByRole("combobox"))
        await user.click(screen.getByRole("option", { name: "PNG" }))
      }
      const fileInput =
        container.querySelector<HTMLInputElement>('input[type="file"]')!
      await user.upload(
        fileInput,
        new File([pngBytes], "20260824095352_489_180.png", {
          type: "image/png",
        })
      )
      await waitFor(() =>
        expect(
          screen.getAllByRole("button", { name: "Download" })
        ).toHaveLength(2)
      )
      for (const button of screen.getAllByRole("button", {
        name: "Download",
      })) {
        await user.click(button)
      }

      expect(downloads).toHaveLength(2)
      for (const download of downloads) {
        expect(download.filename).toBe(filename)
        expect(download.blob.type).toBe("image/png")
        expect(new Uint8Array(await readBlob(download.blob))).toEqual(pngBytes)
      }

      await user.click(screen.getByRole("button", { name: "Clear" }))
      expect(screen.getByRole("button", { name: "Download" })).toBeDisabled()
    }
  )

  it("downloads QR SVG markup from the toolbar instead of the encoded text", async () => {
    const user = userEvent.setup()
    render(
      <NextIntlClientProvider locale="en" messages={enMessages}>
        <QrGenerateTool />
      </NextIntlClientProvider>
    )
    await user.click(screen.getByRole("button", { name: "Example" }))
    await user.click(screen.getByRole("button", { name: "Download" }))
    await user.click(screen.getByRole("button", { name: "Download SVG" }))
    expect(downloads).toHaveLength(2)
    for (const download of downloads) {
      expect(download.filename).toBe("qr.svg")
      expect(download.blob.type).toBe("image/svg+xml;charset=utf-8")
      const xml = new TextDecoder().decode(await readBlob(download.blob))
      const svg = new DOMParser().parseFromString(xml, "image/svg+xml")
      expect(svg.documentElement.localName).toBe("svg")
      expect(svg.querySelector("path")).not.toBeNull()
    }
  })

  it("preserves text downloads for text tools", async () => {
    const user = userEvent.setup()
    render(
      <NextIntlClientProvider locale="en" messages={enMessages}>
        <ToolPageShell
          input=""
          onInputChange={() => {}}
          result="formatted result"
          downloadFilename="result.txt"
        />
      </NextIntlClientProvider>
    )
    await user.click(screen.getByRole("button", { name: "Download" }))
    expect(downloads).toHaveLength(1)
    expect(downloads[0].filename).toBe("result.txt")
    expect(downloads[0].blob.type).toBe("text/plain;charset=utf-8")
    expect(new TextDecoder().decode(await readBlob(downloads[0].blob))).toBe(
      "formatted result"
    )
  })
})
