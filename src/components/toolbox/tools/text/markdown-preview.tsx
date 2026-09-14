"use client"

import { useCallback, useState } from "react"
import ReactMarkdown, { defaultUrlTransform } from "react-markdown"
import remarkGfm from "remark-gfm"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { cn } from "@/lib/utils"

const EXAMPLE = `# Heading

Paragraph with **bold**, *italic*, and \`code\`.

- list item
- [link](https://example.com)

| A | B |
| - | - |
| 1 | 2 |

\`\`\`
code block
\`\`\`
`

const PREVIEW_PROSE =
  "h-full min-h-0 flex-1 overflow-auto rounded-xl border border-border bg-input/30 p-3 text-sm leading-6 break-words " +
  "[&_h1]:mb-2 [&_h1]:text-lg [&_h1]:font-semibold " +
  "[&_h2]:mt-3 [&_h2]:mb-2 [&_h2]:text-base [&_h2]:font-semibold " +
  "[&_h3]:mt-2 [&_h3]:mb-1 [&_h3]:text-sm [&_h3]:font-semibold " +
  "[&_p]:mb-2 [&_p:last-child]:mb-0 " +
  "[&_ul]:mb-2 [&_ul]:list-disc [&_ul]:pl-5 " +
  "[&_ol]:mb-2 [&_ol]:list-decimal [&_ol]:pl-5 " +
  "[&_li]:mb-1 " +
  "[&_code]:rounded [&_code]:bg-muted [&_code]:px-1 [&_code]:font-mono [&_code]:text-xs " +
  "[&_pre]:mb-2 [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:bg-muted [&_pre]:p-2 " +
  "[&_pre_code]:bg-transparent [&_pre_code]:p-0 " +
  "[&_a]:text-primary [&_a]:underline [&_a]:underline-offset-2 " +
  "[&_blockquote]:border-l-2 [&_blockquote]:border-border [&_blockquote]:pl-3 " +
  "[&_hr]:my-2 [&_hr]:border-border " +
  "[&_table]:mb-2 [&_table]:w-full [&_table]:border-collapse " +
  "[&_th]:border [&_th]:border-border [&_th]:px-2 [&_th]:py-1 " +
  "[&_td]:border [&_td]:border-border [&_td]:px-2 [&_td]:py-1"

export default function MarkdownPreviewTool() {
  const [input, setInput] = useState("")
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      example={EXAMPLE}
      result={input}
      downloadFilename="preview.md"
      resultSlot={
        <div className={cn(PREVIEW_PROSE)}>
          <ReactMarkdown
            remarkPlugins={[remarkGfm]}
            skipHtml
            urlTransform={defaultUrlTransform}
            components={{
              img: ({ alt, src }) => {
                const srcText = typeof src === "string" ? src : ""
                return (
                  <span className="text-muted-foreground">
                    {alt || srcText || "image"}
                  </span>
                )
              },
            }}
          >
            {input}
          </ReactMarkdown>
        </div>
      }
    />
  )
}
