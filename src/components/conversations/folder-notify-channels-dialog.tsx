"use client"

import { useCallback, useEffect, useState } from "react"
import { useTranslations } from "next-intl"
import { toast } from "sonner"
import {
  listChatChannels,
  listFolderChatChannels,
  setFolderChatChannels,
} from "@/lib/api"
import { toErrorMessage } from "@/lib/app-error"
import type { ChatChannelInfo } from "@/lib/types"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Label } from "@/components/ui/label"
import { ScrollArea } from "@/components/ui/scroll-area"

export function FolderNotifyChannelsDialog({
  folderId,
  open,
  onOpenChange,
}: {
  folderId: number
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const t = useTranslations("Folder.sidebar.notifyChannelsDialog")
  const tCommon = useTranslations("Folder.common")
  const [channels, setChannels] = useState<ChatChannelInfo[]>([])
  const [selected, setSelected] = useState<Set<number>>(new Set())
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)
  const [loaded, setLoaded] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)

  useEffect(() => {
    if (!open) return
    let cancelled = false
    /* eslint-disable react-hooks/set-state-in-effect -- reset + fetch on open */
    setLoading(true)
    setSaving(false)
    setLoaded(false)
    setLoadError(null)
    void Promise.all([listChatChannels(), listFolderChatChannels(folderId)])
      .then(([listed, bound]) => {
        if (cancelled) return
        setChannels(listed)
        setSelected(new Set(bound))
        setLoaded(true)
      })
      .catch((err) => {
        if (cancelled) return
        // Keep last good channels/selection. Save is a full replace, so an
        // empty fallback here would let a later click wipe every binding.
        const message = toErrorMessage(err)
        setLoadError(message)
        toast.error(t("loadFailed", { message }))
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    /* eslint-enable react-hooks/set-state-in-effect */
    return () => {
      cancelled = true
    }
  }, [open, folderId, t])

  const toggleChannel = useCallback((id: number, checked: boolean) => {
    setSelected((prev) => {
      const next = new Set(prev)
      if (checked) next.add(id)
      else next.delete(id)
      return next
    })
  }, [])

  const handleSave = async () => {
    if (!loaded || saving) return
    setSaving(true)
    const channelIds = channels
      .filter((channel) => selected.has(channel.id))
      .map((channel) => channel.id)
    try {
      await setFolderChatChannels(folderId, channelIds)
      onOpenChange(false)
    } catch (err) {
      toast.error(t("saveFailed", { message: toErrorMessage(err) }))
      setSaving(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[28rem]">
        <DialogHeader>
          <DialogTitle>{t("title")}</DialogTitle>
          <DialogDescription>{t("description")}</DialogDescription>
        </DialogHeader>
        {loading ? (
          <p className="text-sm text-muted-foreground">{tCommon("loading")}</p>
        ) : loadError ? (
          <p className="text-sm text-muted-foreground">
            {t("loadFailed", { message: loadError })}
          </p>
        ) : channels.length === 0 ? (
          <p className="text-sm text-muted-foreground">{t("empty")}</p>
        ) : (
          <ScrollArea className="max-h-64">
            <div className="flex flex-col gap-2 py-1 pr-3">
              {channels.map((channel) => {
                const inputId = `folder-notify-channel-${channel.id}`
                return (
                  <Label
                    key={channel.id}
                    htmlFor={inputId}
                    className="font-normal"
                  >
                    <Checkbox
                      id={inputId}
                      checked={selected.has(channel.id)}
                      onCheckedChange={(value) =>
                        toggleChannel(channel.id, value === true)
                      }
                    />
                    <span className="min-w-0 truncate">{channel.name}</span>
                  </Label>
                )
              })}
            </div>
          </ScrollArea>
        )}
        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={saving}
          >
            {tCommon("cancel")}
          </Button>
          <Button
            type="button"
            onClick={() => void handleSave()}
            disabled={loading || saving || !loaded}
          >
            {tCommon("save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
