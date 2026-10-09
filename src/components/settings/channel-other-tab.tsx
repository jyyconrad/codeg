"use client"

import { useCallback, useEffect, useState } from "react"
import { Loader2 } from "lucide-react"
import { useTranslations } from "next-intl"
import { toast } from "sonner"

import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { getChatMessageLanguage, setChatMessageLanguage } from "@/lib/api"

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

export function ChannelOtherTab() {
  const t = useTranslations("ChatChannelSettings.language")
  const tInbound = useTranslations("ChatChannelSettings.folderInbound")
  const [language, setLanguage] = useState("en")
  const [loading, setLoading] = useState(true)
  const [savingLanguage, setSavingLanguage] = useState(false)

  useEffect(() => {
    let cancelled = false
    void getChatMessageLanguage()
      .catch(() => "en")
      .then((lang) => {
        if (!cancelled) setLanguage(lang)
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
        <h3 className="text-sm font-medium">{tInbound("title")}</h3>
        <p className="text-xs text-muted-foreground">
          {tInbound("description")}
        </p>
      </section>
    </div>
  )
}
