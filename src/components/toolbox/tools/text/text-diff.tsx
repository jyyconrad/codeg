"use client"

import { useCallback, useMemo, useState } from "react"
import { ToolPageShell } from "@/components/toolbox/tool-page-shell"
import { useToolPendingInput } from "@/components/toolbox/use-tool-pending-input"
import { Textarea } from "@/components/ui/textarea"
import { cn } from "@/lib/utils"
import { diffTexts, type DiffRow, type InlinePart } from "./text-diff-core"

const EXAMPLE_ORIGINAL = "foo\nbar\nbaz"
const EXAMPLE_MODIFIED = "foo\nqux\nbaz"

function InlineMarks({
  parts,
  side,
}: {
  parts: InlinePart[]
  side: "old" | "new"
}) {
  return (
    <>
      {parts.map((part, index) => {
        if (part.type === "equal") {
          return <span key={index}>{part.value}</span>
        }
        if (side === "old" && part.type === "remove") {
          return (
            <span key={index} className="bg-destructive/50">
              {part.value}
            </span>
          )
        }
        if (side === "new" && part.type === "add") {
          return (
            <span
              key={index}
              className="bg-emerald-500/40 dark:bg-emerald-500/30"
            >
              {part.value}
            </span>
          )
        }
        return null
      })}
    </>
  )
}

function DiffRows({ rows }: { rows: DiffRow[] }) {
  if (rows.length === 0) {
    return (
      <div className="min-h-[12rem] flex-1 rounded-xl border border-border bg-input/30 p-3 text-sm text-muted-foreground">
        No differences
      </div>
    )
  }
  return (
    <pre className="min-h-[12rem] flex-1 overflow-auto rounded-xl border border-border bg-input/30 p-2 font-mono text-xs leading-5">
      {rows.map((row, index) => {
        if (row.type === "replace") {
          return (
            <div key={index}>
              <div className="bg-destructive/15 px-1 text-destructive">
                <span className="select-none">-</span>
                <InlineMarks parts={row.parts} side="old" />
              </div>
              <div className="bg-emerald-500/10 px-1 text-emerald-800 dark:text-emerald-300">
                <span className="select-none">+</span>
                <InlineMarks parts={row.parts} side="new" />
              </div>
            </div>
          )
        }
        return (
          <div
            key={index}
            className={cn(
              "px-1",
              row.type === "add" &&
                "bg-emerald-500/10 text-emerald-800 dark:text-emerald-300",
              row.type === "remove" && "bg-destructive/15 text-destructive",
              row.type === "equal" && "text-muted-foreground"
            )}
          >
            <span className="select-none">
              {row.type === "add" ? "+" : row.type === "remove" ? "-" : " "}
            </span>
            {row.text}
          </div>
        )
      })}
    </pre>
  )
}

export default function TextDiffTool() {
  const [input, setInput] = useState("")
  const [modified, setModified] = useState("")
  const onInput = useCallback((value: string) => setInput(value), [])
  useToolPendingInput(onInput)

  const { rows, unified } = useMemo(
    () => diffTexts(input, modified),
    [input, modified]
  )

  return (
    <ToolPageShell
      input={input}
      onInputChange={setInput}
      inputLabel="Original / Modified"
      onExample={() => {
        setInput(EXAMPLE_ORIGINAL)
        setModified(EXAMPLE_MODIFIED)
      }}
      result={unified}
      downloadFilename="diff.patch"
      inputSlot={
        <div className="flex min-h-0 flex-1 flex-col gap-2">
          <Textarea
            value={input}
            onChange={(event) => setInput(event.target.value)}
            placeholder="Original"
            className="min-h-[6rem] flex-1 font-mono text-sm"
          />
          <Textarea
            value={modified}
            onChange={(event) => setModified(event.target.value)}
            placeholder="Modified"
            className="min-h-[6rem] flex-1 font-mono text-sm"
          />
        </div>
      }
      resultSlot={<DiffRows rows={rows} />}
    />
  )
}
