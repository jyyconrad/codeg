"use client"

import { ChevronDown, AppWindow, FolderOpen } from "lucide-react"
import { useTranslations } from "next-intl"
import { toast } from "sonner"

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { toErrorMessage } from "@/lib/app-error"
import { isLocalDesktop, openPath, revealItemInDir } from "@/lib/platform"
import { cn } from "@/lib/utils"

/**
 * Header control: open the previewed file with the OS default app, or reveal
 * it in the file manager. Hidden where those openers no-op (web / remote
 * desktop).
 */
export function FileOpenDropdown({
  path,
  className,
}: {
  path: string
  className?: string
}) {
  const t = useTranslations("Folder.fileOpen")

  if (!isLocalDesktop() || !path) return null

  const run = (action: () => Promise<void>) => {
    void action().catch((error) => {
      toast.error(t("failed"), { description: toErrorMessage(error) })
    })
  }

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className={cn(
            "inline-flex h-7 shrink-0 items-center gap-0.5 rounded-md px-1.5 text-xs text-muted-foreground transition-colors hover:bg-foreground/10 hover:text-foreground",
            className
          )}
          aria-label={t("open")}
          title={t("open")}
        >
          {t("open")}
          <ChevronDown className="size-3.5" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-36 w-auto">
        <DropdownMenuItem onSelect={() => run(() => openPath(path))}>
          <AppWindow />
          {t("defaultApp")}
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => run(() => revealItemInDir(path))}>
          <FolderOpen />
          {t("folder")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
