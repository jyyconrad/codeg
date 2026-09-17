"use client"

import { useEffect, useState } from "react"
import { useTranslations } from "next-intl"
import { Loader2 } from "lucide-react"

import { HtmlPreview } from "@/components/files/html-preview"
import { ImagePreview } from "@/components/files/image-preview"
import { MarkdownDocumentPreview } from "@/components/files/markdown-document-preview"
import { OfficeJsPreview } from "@/components/files/office-js-preview"
import { OfficePreview } from "@/components/files/office-preview"
import { SpreadsheetPreview } from "@/components/files/spreadsheet-preview"
import type { FileWorkspaceTab } from "@/contexts/workspace-context"
import { officecliDetect } from "@/lib/api"
import {
  isOfficeCliWatchable,
  previewKindFromPath,
  type FilePreviewKind,
} from "@/lib/file-preview-kind"
import { isDesktop, isRemoteDesktopMode } from "@/lib/transport"

function noop() {}

export function FilePreview(props: {
  path: string
  content: string
  rootPath: string | null
  relPath: string | null
  language: string
  isPreview: boolean
  loading?: boolean
  openFilePreview?: (path: string) => void
  previewRoot?: string | null
  tab?: FileWorkspaceTab
}) {
  const kind = previewKindFromPath(props.path)
  const tab = props.tab ?? stubTab(props)

  if (kind === "image") {
    return <ImagePreview key={tab.id} tab={tab} />
  }
  if (kind === "html") {
    return (
      <HtmlPreview
        key={tab.id}
        tab={tab}
        rootPath={props.previewRoot ?? props.rootPath}
      />
    )
  }
  if (kind === "markdown") {
    return (
      <MarkdownDocumentPreview
        content={props.content}
        fileDir={props.rootPath}
        previewRoot={props.previewRoot ?? props.rootPath}
        openFilePreview={props.openFilePreview ?? noop}
      />
    )
  }
  if (
    kind === "pdf" ||
    kind === "docx" ||
    kind === "pptx" ||
    kind === "spreadsheet"
  ) {
    return (
      <OfficeEnginePreview
        kind={kind}
        path={props.path}
        rootPath={props.rootPath}
        relPath={props.relPath}
      />
    )
  }
  return null
}

function stubTab(props: {
  path: string
  content: string
  language: string
  loading?: boolean
}): FileWorkspaceTab {
  const title = props.path.split(/[\\/]/).pop() ?? props.path
  return {
    id: props.path,
    kind: "file",
    folderId: null,
    title,
    description: props.path,
    path: props.path,
    language: props.language,
    content: props.content,
    loading: props.loading ?? false,
  }
}

function OfficeEnginePreview({
  kind,
  path,
  rootPath,
  relPath,
}: {
  kind: Extract<FilePreviewKind, "pdf" | "docx" | "pptx" | "spreadsheet">
  path: string
  rootPath: string | null
  relPath: string | null
}) {
  const t = useTranslations("Folder.fileWorkspacePanel")
  const remoteDesktop = isDesktop() && isRemoteDesktopMode()
  const watchable = isOfficeCliWatchable(path) && !remoteDesktop
  const [forceFallback, setForceFallback] = useState(false)
  const [cli, setCli] = useState<"loading" | "yes" | "no">(
    watchable ? "loading" : "no"
  )

  useEffect(() => {
    if (!watchable || forceFallback) {
      setCli("no")
      return
    }
    let cancelled = false
    officecliDetect()
      .then((info) => {
        if (cancelled) return
        setCli(info.installed && !info.runtimeError ? "yes" : "no")
      })
      .catch(() => {
        if (!cancelled) setCli("no")
      })
    return () => {
      cancelled = true
    }
  }, [watchable, forceFallback, path])

  if (watchable && !forceFallback && cli === "loading") {
    return (
      <div className="flex h-full items-center justify-center gap-2 text-xs text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" />
        {t("loading")}
      </div>
    )
  }

  if (watchable && !forceFallback && cli === "yes") {
    return (
      <OfficePreview
        rootPath={rootPath}
        relPath={relPath}
        onNotInstalled={() => setForceFallback(true)}
      />
    )
  }

  if (kind === "spreadsheet") {
    return (
      <SpreadsheetPreview
        path={path}
        rootPath={rootPath}
        relPath={relPath}
      />
    )
  }

  return (
    <OfficeJsPreview
      kind={kind}
      path={path}
      rootPath={rootPath}
      relPath={relPath}
    />
  )
}
