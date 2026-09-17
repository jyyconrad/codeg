/**
 * Image previews portal to the body. Conversation tabs stay mounted when
 * backgrounded, so without the host-hidden flag a preview opened in A keeps
 * painting over B. Hidden, not closed: switching back must find the same
 * picture still open.
 */
import { act, render, screen } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"

import { OverlayHostHiddenProvider } from "@/components/ui/overlay-host-hidden"
import { ImagePreviewDialog } from "@/components/ui/image-preview-dialog"

const SRC = "data:image/png;base64,aaaa"

async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
}

describe("ImagePreviewDialog under a hidden host surface", () => {
  it("stops painting without closing, and restores when the host is shown again", async () => {
    const onOpenChange = vi.fn()
    const { rerender } = render(
      <OverlayHostHiddenProvider hidden={false}>
        <ImagePreviewDialog
          src={SRC}
          alt="attachment-a"
          open
          onOpenChange={onOpenChange}
        />
      </OverlayHostHiddenProvider>
    )
    await settle()
    expect(screen.getByAltText("attachment-a")).toBeInTheDocument()

    rerender(
      <OverlayHostHiddenProvider hidden>
        <ImagePreviewDialog
          src={SRC}
          alt="attachment-a"
          open
          onOpenChange={onOpenChange}
        />
      </OverlayHostHiddenProvider>
    )
    await settle()

    expect(screen.queryByAltText("attachment-a")).not.toBeInTheDocument()
    // Hidden, NOT closed: parent state stays so switching back restores it.
    expect(onOpenChange).not.toHaveBeenCalled()

    rerender(
      <OverlayHostHiddenProvider hidden={false}>
        <ImagePreviewDialog
          src={SRC}
          alt="attachment-a"
          open
          onOpenChange={onOpenChange}
        />
      </OverlayHostHiddenProvider>
    )
    await settle()
    expect(screen.getByAltText("attachment-a")).toBeInTheDocument()
  })

  it("shows the visible conversation's preview while the backgrounded one stays hidden", async () => {
    render(
      <>
        <OverlayHostHiddenProvider hidden>
          <ImagePreviewDialog
            src={SRC}
            alt="attachment-a"
            open
            onOpenChange={vi.fn()}
          />
        </OverlayHostHiddenProvider>
        <OverlayHostHiddenProvider hidden={false}>
          <ImagePreviewDialog
            src={SRC}
            alt="attachment-b"
            open
            onOpenChange={vi.fn()}
          />
        </OverlayHostHiddenProvider>
      </>
    )
    await settle()

    expect(screen.queryByAltText("attachment-a")).not.toBeInTheDocument()
    expect(screen.getByAltText("attachment-b")).toBeInTheDocument()
  })
})
