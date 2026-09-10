import { diffChars, diffLines } from "diff"

export type InlinePart = {
  value: string
  type: "equal" | "add" | "remove"
}

export type DiffRow =
  | { type: "equal"; text: string }
  | { type: "add"; text: string }
  | { type: "remove"; text: string }
  | {
      type: "replace"
      before: string
      after: string
      parts: InlinePart[]
    }

function splitDiffValue(value: string): string[] {
  if (value === "") return []
  const parts = value.split("\n")
  if (parts[parts.length - 1] === "") parts.pop()
  return parts
}

function inlineParts(before: string, after: string): InlinePart[] {
  return diffChars(before, after).map((part) => ({
    value: part.value,
    type: part.added ? "add" : part.removed ? "remove" : "equal",
  }))
}

export function formatUnifiedDiff(rows: DiffRow[]): string {
  const out: string[] = []
  for (const row of rows) {
    if (row.type === "equal") out.push(` ${row.text}`)
    else if (row.type === "add") out.push(`+${row.text}`)
    else if (row.type === "remove") out.push(`-${row.text}`)
    else {
      out.push(`-${row.before}`)
      out.push(`+${row.after}`)
    }
  }
  return out.join("\n")
}

export function diffTexts(
  original: string,
  modified: string
): { rows: DiffRow[]; unified: string } {
  if (original === "" && modified === "") {
    return { rows: [], unified: "" }
  }
  const changes = diffLines(original, modified)
  const rows: DiffRow[] = []
  for (let i = 0; i < changes.length; i += 1) {
    const change = changes[i]
    const next = changes[i + 1]
    if (change.removed && next?.added) {
      const oldLines = splitDiffValue(change.value)
      const newLines = splitDiffValue(next.value)
      const paired = Math.min(oldLines.length, newLines.length)
      for (let j = 0; j < paired; j += 1) {
        if (oldLines[j] === newLines[j]) {
          rows.push({ type: "equal", text: oldLines[j] })
        } else {
          rows.push({
            type: "replace",
            before: oldLines[j],
            after: newLines[j],
            parts: inlineParts(oldLines[j], newLines[j]),
          })
        }
      }
      for (let j = paired; j < oldLines.length; j += 1) {
        rows.push({ type: "remove", text: oldLines[j] })
      }
      for (let j = paired; j < newLines.length; j += 1) {
        rows.push({ type: "add", text: newLines[j] })
      }
      i += 1
      continue
    }
    const kind = change.added ? "add" : change.removed ? "remove" : "equal"
    for (const line of splitDiffValue(change.value)) {
      rows.push({ type: kind, text: line })
    }
  }
  return { rows, unified: formatUnifiedDiff(rows) }
}
