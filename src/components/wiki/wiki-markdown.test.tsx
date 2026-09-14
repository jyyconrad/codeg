import { render, screen, fireEvent } from "@testing-library/react"
import { NextIntlClientProvider } from "next-intl"
import { describe, it, expect, vi } from "vitest"
import { WikiMarkdownPreview } from "./wiki-shared"
import messages from "@/i18n/messages/en.json"
vi.mock("@/components/ai-elements/markdown-link", () => ({
  MarkdownLink: ({
    href,
    children,
  }: {
    href: string
    children: React.ReactNode
  }) => <a href={href}>{children}</a>,
}))

describe("Wiki Markdown reader", () => {
  it("renders Markdown and working links while keeping code and hiding metadata", () => {
    const open = vi.fn()
    render(
      <NextIntlClientProvider locale="en" messages={messages}>
        <WikiMarkdownPreview
          content={
            "---\ntitle: metadata-secret\n---\n# Reading\n\n**Strong**\n\n<!-- comment-secret -->\n\n[[work/next|Next note]]\n\n`[[work/code]]`\n\n```md\n[[work/fenced]]\n<!-- example -->\n```\n\n[Relative](../methods.md#Checks)"
          }
          path="work/current.md"
          onOpenWikilink={open}
        />
      </NextIntlClientProvider>
    )
    expect(screen.getByRole("heading", { name: "Reading" })).toBeVisible()
    expect(screen.getByText("Strong").tagName).toBe("STRONG")
    expect(
      screen.queryByText(/metadata-secret|comment-secret/)
    ).not.toBeInTheDocument()
    expect(screen.getByText("[[work/code]]").tagName).toBe("CODE")
    expect(screen.getByText(/work\/fenced/).tagName).toBe("CODE")
    fireEvent.click(screen.getByRole("link", { name: "Next note" }))
    expect(open).toHaveBeenLastCalledWith("work/next.md", "")
    fireEvent.click(screen.getByRole("link", { name: "Relative" }))
    expect(open).toHaveBeenLastCalledWith("methods.md", "checks")
  })
})
