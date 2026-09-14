"use client"

import { useEffect, useState } from "react"
import {
  guardImageFile,
  guardImageSize,
  IMAGE_DECODE_MESSAGE,
  loadImageFromUrl,
} from "./image-io"
import { useObjectUrl } from "./use-object-url"

export function useLoadedImage(file: File | null): {
  image: HTMLImageElement | null
  previewUrl: string | null
  error: string | null
} {
  const sizeErr = file ? guardImageFile(file) : null
  const previewUrl = useObjectUrl(sizeErr ? null : file)
  const [loaded, setLoaded] = useState<{
    file: File
    image: HTMLImageElement
  } | null>(null)
  const [error, setError] = useState<{ file: File; message: string } | null>(
    null
  )

  useEffect(() => {
    if (!file || sizeErr || !previewUrl) return
    let cancelled = false
    void loadImageFromUrl(previewUrl)
      .then((img) => {
        if (cancelled) return
        const dimErr = guardImageSize(img.naturalWidth, img.naturalHeight)
        if (dimErr) {
          setError({ file, message: dimErr })
          return
        }
        setLoaded({ file, image: img })
        setError(null)
      })
      .catch((err: unknown) => {
        if (cancelled) return
        setError({
          file,
          message: err instanceof Error ? err.message : IMAGE_DECODE_MESSAGE,
        })
      })
    return () => {
      cancelled = true
    }
  }, [file, previewUrl, sizeErr])

  return {
    image: file && loaded?.file === file ? loaded.image : null,
    previewUrl: file && !sizeErr ? previewUrl : null,
    error: file
      ? (sizeErr ?? (error?.file === file ? error.message : null))
      : null,
  }
}
