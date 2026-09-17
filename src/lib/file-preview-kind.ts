import {
  isHtmlPreviewable,
  isImageFile,
  isOfficePreviewable,
  languageFromPath,
} from "@/lib/language-detect"

export type FilePreviewKind =
  | "pdf"
  | "docx"
  | "pptx"
  | "spreadsheet"
  | "image"
  | "html"
  | "markdown"
  | "source"

function extensionOf(path: string): string {
  const basename = path.toLowerCase().split(/[\\/]/).pop() ?? ""
  const dot = basename.lastIndexOf(".")
  if (dot <= 0 || dot === basename.length - 1) return ""
  return basename.slice(dot + 1)
}

export function previewKindFromPath(path: string): FilePreviewKind {
  const ext = extensionOf(path)
  if (ext === "pdf") return "pdf"
  if (ext === "docx") return "docx"
  if (ext === "pptx") return "pptx"
  if (ext === "xlsx" || ext === "xls" || ext === "csv") return "spreadsheet"
  if (isImageFile(path)) return "image"
  if (isHtmlPreviewable(path)) return "html"
  if (ext === "md" || ext === "markdown") return "markdown"
  return "source"
}

export function isCsvPath(path: string | null | undefined): boolean {
  return !!path && extensionOf(path) === "csv"
}

/** Binary tabs: never read as UTF-8, never shown in Monaco. CSV is text. */
export function isBinaryPreviewable(path: string | null | undefined): boolean {
  if (!path) return false
  const kind = previewKindFromPath(path)
  return (
    kind === "pdf" ||
    kind === "docx" ||
    kind === "pptx" ||
    (kind === "spreadsheet" && !isCsvPath(path))
  )
}

/** Formats `officecli watch` can render. PDF / .xls / CSV are not among them. */
export function isOfficeCliWatchable(path: string | null | undefined): boolean {
  return isOfficePreviewable(path)
}

export function hasSourcePreviewToggle(
  path: string | null | undefined
): boolean {
  if (!path) return false
  const kind = previewKindFromPath(path)
  return kind === "markdown" || kind === "html" || isCsvPath(path)
}

export function shouldUseFilePreview(path: string, isPreview: boolean): boolean {
  const kind = previewKindFromPath(path)
  if (kind === "source") return false
  if (kind === "html" || kind === "markdown") return isPreview
  if (kind === "spreadsheet" && isCsvPath(path)) return isPreview
  return true
}

export function tabLanguageFromPath(path: string): string {
  if (isImageFile(path)) return "image"
  const kind = previewKindFromPath(path)
  if (kind === "pdf") return "pdf"
  if (kind === "docx" || kind === "pptx") return "office"
  if (kind === "spreadsheet" && !isCsvPath(path)) return "spreadsheet"
  return languageFromPath(path)
}
