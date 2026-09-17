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
import { cn } from "@/lib/utils"
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
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { ScrollArea } from "@/components/ui/scroll-area"

function channelSupportsSessionId(channel: ChatChannelInfo): boolean {
  return channel.channel_type === "telegram" || channel.channel_type === "lark"
}

function channelDefaultChatId(channel: ChatChannelInfo): string | null {
  try {
    const config = JSON.parse(channel.config_json || "{}") as {
      chat_id?: unknown
    }
    return typeof config.chat_id === "string" && config.chat_id.trim()
      ? config.chat_id.trim()
      : null
  } catch {
    return null
  }
}

function normalizeSessionId(value: string | undefined): string | null {
  const trimmed = value?.trim()
  return trimmed ? trimmed : null
}

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
  const [sessionIds, setSessionIds] = useState<Record<number, string>>({})
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
        const enabled = listed.filter((channel) => channel.enabled)
        const nextSelected = new Set<number>()
        const nextSessionIds: Record<number, string> = {}
        for (const binding of bound) {
          if (!enabled.some((channel) => channel.id === binding.channel_id)) {
            continue
          }
          nextSelected.add(binding.channel_id)
          const chatId = binding.chat_id?.trim()
          if (chatId) nextSessionIds[binding.channel_id] = chatId
        }
        setChannels(enabled)
        setSelected(nextSelected)
        setSessionIds(nextSessionIds)
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

  const handleSessionIdChange = useCallback((id: number, value: string) => {
    setSessionIds((prev) => ({ ...prev, [id]: value }))
  }, [])

  const handleSave = async () => {
    if (!loaded || saving) return
    setSaving(true)
    const channelsToSave = channels
      .filter((channel) => selected.has(channel.id))
      .map((channel) => ({
        channel_id: channel.id,
        chat_id: channelSupportsSessionId(channel)
          ? normalizeSessionId(sessionIds[channel.id])
          : null,
      }))
    try {
      await setFolderChatChannels(folderId, channelsToSave)
      onOpenChange(false)
    } catch (err) {
      toast.error(t("saveFailed", { message: toErrorMessage(err) }))
      setSaving(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
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
          <ScrollArea className="max-h-80">
            <div className="flex flex-col gap-2 py-1 pr-3">
              {channels.map((channel) => {
                const checkboxId = `folder-notify-channel-${channel.id}`
                const sessionInputId = `folder-notify-session-${channel.id}`
                const checked = selected.has(channel.id)
                const defaultChatId = channelDefaultChatId(channel)
                return (
                  <div
                    key={channel.id}
                    className={cn(
                      "rounded-xl border border-border/70 p-3",
                      checked && "bg-muted/30"
                    )}
                  >
                    <Label htmlFor={checkboxId} className="font-normal">
                      <Checkbox
                        id={checkboxId}
                        checked={checked}
                        onCheckedChange={(value) =>
                          toggleChannel(channel.id, value === true)
                        }
                      />
                      <span className="min-w-0 truncate">{channel.name}</span>
                    </Label>
                    {checked && channelSupportsSessionId(channel) ? (
                      <div className="mt-3 space-y-1.5 pl-6">
                        <label
                          htmlFor={sessionInputId}
                          className="text-xs font-medium"
                        >
                          {t("sessionId")}
                        </label>
                        <Input
                          id={sessionInputId}
                          value={sessionIds[channel.id] ?? ""}
                          onChange={(event) =>
                            handleSessionIdChange(
                              channel.id,
                              event.target.value
                            )
                          }
                          aria-label={t("sessionIdForChannel", {
                            name: channel.name,
                          })}
                          placeholder={
                            defaultChatId
                              ? t("sessionIdPlaceholderWithDefault", {
                                  chatId: defaultChatId,
                                })
                              : t("sessionIdPlaceholder")
                          }
                          autoComplete="off"
                          spellCheck={false}
                        />
                        <p className="text-xs text-muted-foreground">
                          {t("sessionIdHint")}
                        </p>
                      </div>
                    ) : null}
                  </div>
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
