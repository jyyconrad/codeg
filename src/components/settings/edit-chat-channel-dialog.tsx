"use client"

import { useCallback, useEffect, useId, useState } from "react"
import { Loader2 } from "lucide-react"
import { useTranslations } from "next-intl"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { useAcpAgents } from "@/hooks/use-acp-agents"
import {
  updateChatChannel,
  saveChatChannelToken,
  getChatChannelHasToken,
} from "@/lib/api"
import { filterAvailableAgents } from "@/lib/available-agents"
import type { ChatChannelInfo } from "@/lib/types"
import { toErrorMessage } from "@/lib/app-error"

interface EditChatChannelDialogProps {
  open: boolean
  channel: ChatChannelInfo
  onOpenChange: (open: boolean) => void
  onChannelUpdated: () => void
}

export function EditChatChannelDialog({
  open,
  channel,
  onOpenChange,
  onChannelUpdated,
}: EditChatChannelDialogProps) {
  const t = useTranslations("ChatChannelSettings")
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const config = JSON.parse(channel.config_json || "{}")
  const [name, setName] = useState(channel.name)
  const [token, setToken] = useState("")
  const [chatId, setChatId] = useState(config.chat_id ?? "")
  const [appId, setAppId] = useState(config.app_id ?? "")
  const [baseUrl] = useState(config.base_url ?? "")
  const [topicMode, setTopicMode] = useState(Boolean(config.topic_mode))
  const [dailyReportEnabled, setDailyReportEnabled] = useState(
    channel.daily_report_enabled
  )
  const [dailyReportTime, setDailyReportTime] = useState(
    channel.daily_report_time || "18:00"
  )
  const [hasToken, setHasToken] = useState(false)
  const topicModeId = useId()
  const noneAgent = "__none__"
  const { agents } = useAcpAgents()
  const availableAgents = filterAvailableAgents(agents)
  const [defaultAgent, setDefaultAgent] = useState(
    channel.default_agent_type?.trim() || noneAgent
  )
  const agentChoices =
    defaultAgent !== noneAgent &&
    !availableAgents.some((agent) => agent.agent_type === defaultAgent)
      ? [
          ...availableAgents,
          {
            agent_type: defaultAgent,
            name: defaultAgent,
          },
        ]
      : availableAgents

  useEffect(() => {
    if (open) {
      getChatChannelHasToken(channel.id)
        .then(setHasToken)
        .catch(() => {})
    }
  }, [open, channel.id])

  const handleSubmit = useCallback(async () => {
    if (!name.trim()) {
      setError(t("nameRequired"))
      return
    }
    if (channel.channel_type !== "weixin" && !chatId.trim()) {
      setError(t("chatIdRequired"))
      return
    }

    setLoading(true)
    setError(null)
    try {
      const configJson =
        channel.channel_type === "weixin"
          ? JSON.stringify({ base_url: baseUrl })
          : channel.channel_type === "lark"
            ? JSON.stringify({ app_id: appId, chat_id: chatId })
            : JSON.stringify({ chat_id: chatId, topic_mode: topicMode })

      await updateChatChannel({
        id: channel.id,
        name: name.trim(),
        configJson,
        dailyReportEnabled,
        dailyReportTime: dailyReportEnabled ? dailyReportTime : null,
        defaultAgentType: defaultAgent === noneAgent ? null : defaultAgent,
      })

      if (token.trim()) {
        await saveChatChannelToken(channel.id, token.trim())
      }

      onOpenChange(false)
      onChannelUpdated()
      toast.success(t("editSuccess"))
    } catch (err: unknown) {
      const msg = toErrorMessage(err)
      setError(msg)
    } finally {
      setLoading(false)
    }
  }, [
    name,
    token,
    chatId,
    channel,
    appId,
    baseUrl,
    topicMode,
    dailyReportEnabled,
    dailyReportTime,
    defaultAgent,
    noneAgent,
    onOpenChange,
    onChannelUpdated,
    t,
  ])

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("editChannel")}</DialogTitle>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-1.5">
            <label className="text-xs font-medium">{t("channelName")}</label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t("channelNamePlaceholder")}
            />
          </div>

          {channel.channel_type === "lark" && (
            <div className="space-y-1.5">
              <label className="text-xs font-medium">App ID</label>
              <Input
                value={appId}
                onChange={(e) => setAppId(e.target.value)}
                placeholder="cli_xxxxx"
              />
            </div>
          )}

          {channel.channel_type !== "weixin" && (
            <div className="space-y-1.5">
              <label className="text-xs font-medium">
                {channel.channel_type === "telegram"
                  ? "Bot Token"
                  : "App Secret"}
              </label>
              <Input
                type="password"
                value={token}
                onChange={(e) => setToken(e.target.value)}
                placeholder={
                  hasToken ? t("tokenPlaceholderKeep") : t("tokenRequired")
                }
              />
            </div>
          )}

          {channel.channel_type !== "weixin" && (
            <div className="space-y-1.5">
              <label className="text-xs font-medium">Chat ID</label>
              <Input
                value={chatId}
                onChange={(e) => setChatId(e.target.value)}
                placeholder={
                  channel.channel_type === "telegram"
                    ? "-100123456789"
                    : "oc_xxxxx"
                }
              />
            </div>
          )}

          {channel.channel_type === "telegram" && (
            <div className="rounded-md border border-border/70 p-3">
              <div className="flex items-center justify-between gap-3">
                <div>
                  <label htmlFor={topicModeId} className="text-xs font-medium">
                    {t("topicMode")}
                  </label>
                  <p className="mt-1 text-xs text-muted-foreground">
                    {t("topicModeHint")}
                  </p>
                </div>
                <Switch
                  id={topicModeId}
                  checked={topicMode}
                  onCheckedChange={setTopicMode}
                />
              </div>
            </div>
          )}

          {channel.channel_type === "weixin" && baseUrl && (
            <div className="space-y-1.5">
              <label className="text-xs font-medium">Base URL</label>
              <Input value={baseUrl} disabled />
            </div>
          )}

          <div className="space-y-1.5">
            <label className="text-xs font-medium">{t("defaultAgent")}</label>
            <Select value={defaultAgent} onValueChange={setDefaultAgent}>
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={noneAgent}>
                  {t("defaultAgentNone")}
                </SelectItem>
                {agentChoices.map((agent) => (
                  <SelectItem key={agent.agent_type} value={agent.agent_type}>
                    {agent.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">
              {availableAgents.length === 0
                ? t("defaultAgentEmpty")
                : t("defaultAgentHint")}
            </p>
          </div>

          <div className="flex items-center justify-between">
            <label className="text-xs font-medium">{t("dailyReport")}</label>
            <Switch
              checked={dailyReportEnabled}
              onCheckedChange={setDailyReportEnabled}
            />
          </div>

          {dailyReportEnabled && (
            <div className="space-y-1.5">
              <label className="text-xs font-medium">
                {t("dailyReportTime")}
              </label>
              <Input
                type="time"
                value={dailyReportTime}
                onChange={(e) => setDailyReportTime(e.target.value)}
              />
            </div>
          )}

          {error && (
            <div className="rounded-md border border-red-500/30 bg-red-500/5 px-3 py-2 text-xs text-red-400">
              {error}
            </div>
          )}
        </div>

        <DialogFooter>
          <Button
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={loading}
          >
            {t("cancel")}
          </Button>
          <Button onClick={handleSubmit} disabled={loading}>
            {loading && <Loader2 className="h-3.5 w-3.5 animate-spin mr-1" />}
            {t("save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
