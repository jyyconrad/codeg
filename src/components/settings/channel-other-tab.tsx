"use client"

import { useCallback, useEffect, useState } from "react"
import { Loader2, Save } from "lucide-react"
import { useTranslations } from "next-intl"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  getChatFolderInboundIdleMinutes,
  getChatMessageLanguage,
  setChatFolderInboundIdleMinutes,
  setChatMessageLanguage,
} from "@/lib/api"

const SUPPORTED_LANGUAGES = [
  "en",
  "zh-cn",
  "zh-tw",
  "ja",
  "ko",
  "es",
  "de",
  "fr",
  "pt",
  "ar",
] as const

const DEFAULT_IDLE_MINUTES = 30
const MAX_IDLE_MINUTES = 7 * 24 * 60

export function parseIdleMinutesInput(raw: string): number | null {
  const trimmed = raw.trim()
  if (!/^\d+$/.test(trimmed)) return null
  const minutes = Number(trimmed)
  if (!Number.isInteger(minutes) || minutes < 0 || minutes > MAX_IDLE_MINUTES) {
    return null
  }
  return minutes
}

export function ChannelOtherTab() {
  const t = useTranslations("ChatChannelSettings.language")
  const tIdle = useTranslations("ChatChannelSettings.folderInbound")
  const [language, setLanguage] = useState("en")
  const [minutes, setMinutes] = useState(DEFAULT_IDLE_MINUTES)
  const [inputMinutes, setInputMinutes] = useState(String(DEFAULT_IDLE_MINUTES))
  const [loading, setLoading] = useState(true)
  const [savingLanguage, setSavingLanguage] = useState(false)
  const [savingIdle, setSavingIdle] = useState(false)

  useEffect(() => {
    let cancelled = false
    void Promise.all([
      getChatMessageLanguage().catch(() => "en"),
      getChatFolderInboundIdleMinutes().catch(() => DEFAULT_IDLE_MINUTES),
    ])
      .then(([lang, idleMinutes]) => {
        if (cancelled) return
        setLanguage(lang)
        setMinutes(idleMinutes)
        setInputMinutes(String(idleMinutes))
      })
      .finally(() => {
        if (!cancelled) setLoading(false)
      })
    return () => {
      cancelled = true
    }
  }, [])

  const handleLanguageChange = useCallback(
    async (value: string) => {
      setSavingLanguage(true)
      try {
        await setChatMessageLanguage(value)
        setLanguage(value)
        toast.success(t("saved"))
      } catch {
        toast.error(t("saveFailed"))
      } finally {
        setSavingLanguage(false)
      }
    },
    [t]
  )

  const handleSaveIdle = useCallback(async () => {
    const parsed = parseIdleMinutesInput(inputMinutes)
    if (parsed === null) {
      toast.error(tIdle("invalid"))
      return
    }
    setSavingIdle(true)
    try {
      await setChatFolderInboundIdleMinutes(parsed)
      setMinutes(parsed)
      setInputMinutes(String(parsed))
      toast.success(tIdle("saved"))
    } catch {
      toast.error(tIdle("saveFailed"))
    } finally {
      setSavingIdle(false)
    }
  }, [inputMinutes, tIdle])

  const idleDirty = inputMinutes.trim() !== String(minutes)

  if (loading) {
    return (
      <div className="h-full flex items-center justify-center text-sm text-muted-foreground gap-2">
        <Loader2 className="h-4 w-4 animate-spin" />
      </div>
    )
  }

  return (
    <div className="space-y-6">
      <section className="space-y-2">
        <h3 className="text-sm font-medium">{t("title")}</h3>
        <p className="text-xs text-muted-foreground">{t("description")}</p>
        <Select
          value={language}
          onValueChange={handleLanguageChange}
          disabled={savingLanguage}
        >
          <SelectTrigger className="w-56">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {SUPPORTED_LANGUAGES.map((lang) => (
              <SelectItem key={lang} value={lang}>
                {t(lang)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </section>

      <section className="space-y-2">
        <h3 className="text-sm font-medium">{tIdle("title")}</h3>
        <p className="text-xs text-muted-foreground">{tIdle("description")}</p>
        <div className="flex items-center gap-2">
          <Input
            type="number"
            min={0}
            max={MAX_IDLE_MINUTES}
            step={1}
            value={inputMinutes}
            onChange={(e) => setInputMinutes(e.target.value)}
            className="w-24"
            aria-label={tIdle("title")}
          />
          <span className="text-xs text-muted-foreground">
            {tIdle("minutesLabel")}
          </span>
          <Button
            size="sm"
            disabled={!idleDirty || savingIdle}
            onClick={handleSaveIdle}
          >
            {savingIdle ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Save className="h-3.5 w-3.5 mr-1" />
            )}
            {tIdle("save")}
          </Button>
        </div>
      </section>
    </div>
  )
}
