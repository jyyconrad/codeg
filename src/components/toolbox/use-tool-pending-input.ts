"use client"

import { useEffect } from "react"
import { useToolboxStore } from "./toolbox-store"

/** Apply a chained result once when this tool becomes the active destination. */
export function useToolPendingInput(onInput: (value: string) => void): void {
  const consumePendingInput = useToolboxStore((s) => s.consumePendingInput)
  useEffect(() => {
    const pending = consumePendingInput()
    if (pending != null) onInput(pending)
  }, [consumePendingInput, onInput])
}
