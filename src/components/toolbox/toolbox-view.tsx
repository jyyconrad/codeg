"use client"

import { useEffect, useState } from "react"
import { ToolboxCatalog } from "./toolbox-catalog"
import { ToolboxWorkspace } from "./toolbox-workspace"
import { useToolboxStore } from "./toolbox-store"

export function ToolboxView() {
  const hydrate = useToolboxStore((s) => s.hydrate)
  const [query, setQuery] = useState("")

  useEffect(() => {
    hydrate()
  }, [hydrate])

  return (
    <div className="flex h-full min-h-0 w-full">
      <ToolboxCatalog query={query} onQueryChange={setQuery} />
      <ToolboxWorkspace />
    </div>
  )
}
