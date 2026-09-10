"use client"

import { useEffect, useState } from "react"

export function useObjectUrl(source: Blob | null): string | null {
  const [entry, setEntry] = useState<{ source: Blob; url: string } | null>(null)

  useEffect(() => {
    if (!source) return
    const url = URL.createObjectURL(source)
    const frame = requestAnimationFrame(() => {
      setEntry({ source, url })
    })
    return () => {
      cancelAnimationFrame(frame)
      URL.revokeObjectURL(url)
    }
  }, [source])

  if (!source) return null
  return entry?.source === source ? entry.url : null
}
