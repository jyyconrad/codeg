import { listToolboxTools } from "./registry"
import type { ToolboxToolMeta } from "./types"

export interface ToolboxSearchLabels {
  title: string
  description: string
  aliases: string
  category: string
}

function normalize(value: string): string {
  return value.trim().toLowerCase().replace(/\s+/g, "")
}

export function matchesToolboxQuery(
  tool: ToolboxToolMeta,
  labels: ToolboxSearchLabels,
  query: string
): boolean {
  const needle = normalize(query)
  if (!needle) return true
  const haystack = [
    tool.id,
    tool.category,
    ...tool.aliases,
    labels.title,
    labels.description,
    labels.aliases,
    labels.category,
  ]
    .map(normalize)
    .join("\n")
  return haystack.includes(needle)
}

export function searchToolboxTools(
  query: string,
  labelFor: (tool: ToolboxToolMeta) => ToolboxSearchLabels
): ToolboxToolMeta[] {
  return listToolboxTools().filter((tool) =>
    matchesToolboxQuery(tool, labelFor(tool), query)
  )
}
