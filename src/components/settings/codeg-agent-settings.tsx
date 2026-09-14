"use client"

import { useCallback, useEffect, useMemo, useState } from "react"
import {
  Cpu,
  Loader2,
  MessageSquareText,
  Save,
  SlidersHorizontal,
} from "lucide-react"
import { useTranslations } from "next-intl"
import { toast } from "sonner"

import { SettingCard, SettingRow } from "@/components/shared/setting-card"
import {
  SettingsError,
  SettingsSaveBar,
  SettingsSection,
} from "@/components/shared/settings-section"
import {
  CodegAgentCompactModelField,
  CodegAgentPromptEditors,
} from "@/components/settings/codeg-agent-fields"
import { CodegAgentProviderManager } from "@/components/settings/codeg-agent-provider-manager"
import { Input } from "@/components/ui/input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Switch } from "@/components/ui/switch"
import { toErrorMessage } from "@/lib/app-error"
import {
  acpListAgents,
  acpPreflight,
  acpUpdateAgentEnv,
  listModelProviders,
} from "@/lib/api"
import {
  CODEG_BIND_SELECT_ID,
  CODEG_COMPACT_RECENT_TURNS_KEY,
  CODEG_COMPACT_SOFT_PERCENT_KEY,
  CODEG_DEFAULT_COMPACT_RECENT_TURNS,
  CODEG_DEFAULT_COMPACT_SOFT_PERCENT,
  CODEG_DEFAULT_MAX_TURNS,
  CODEG_MAX_TURNS_KEY,
  CODEG_WINDOW_INPUT_ID,
  bindCodegProviderEnv,
  codegDraftFromEnv,
  codegEnvInt,
  codegMaxOutputTokens,
  overlayCodegPromptEnv,
  patchCodegEnvInt,
  patchCodegMaxOutputTokens,
  persistThenRunPreflight,
} from "@/lib/codeg-agent-env"
import { modelProvidersForAgent } from "@/lib/codeg-agent-providers"
import {
  CODEG_BUILTIN_COMPACT_PROMPT,
  CODEG_BUILTIN_SYSTEM_PROMPT,
} from "@/lib/codeg-agent-prompts"
import { parseEnvText } from "@/lib/env-text"
import type {
  AcpAgentInfo,
  ModelProviderInfo,
  PreflightResult,
} from "@/lib/types"

export function CodegAgentSettings() {
  const t = useTranslations("CodegAgentSettings")
  const tAgent = useTranslations("AcpAgentSettings.codegAgent")
  const tAcp = useTranslations("AcpAgentSettings")

  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [agent, setAgent] = useState<AcpAgentInfo | null>(null)
  const [providers, setProviders] = useState<ModelProviderInfo[]>([])
  const [preflight, setPreflight] = useState<PreflightResult | null>(null)
  const [selectedProviderId, setSelectedProviderId] = useState<number | null>(
    null
  )

  const [enabled, setEnabled] = useState(false)
  const [envText, setEnvText] = useState("")
  const [modelProviderId, setModelProviderId] = useState<number | null>(null)
  const [systemPrompt, setSystemPrompt] = useState(CODEG_BUILTIN_SYSTEM_PROMPT)
  const [compactPrompt, setCompactPrompt] = useState(
    CODEG_BUILTIN_COMPACT_PROMPT
  )

  const bindableProviders = useMemo(
    () => modelProvidersForAgent("codeg_agent", providers),
    [providers]
  )

  const boundProvider = useMemo(
    () =>
      bindableProviders.find((provider) => provider.id === modelProviderId) ??
      null,
    [bindableProviders, modelProviderId]
  )

  const applyAgent = useCallback(
    (next: AcpAgentInfo, rows: ModelProviderInfo[]) => {
      const draft = codegDraftFromEnv(next.env)
      const bound =
        rows.find((row) => row.id === next.model_provider_id) ?? null
      setAgent(next)
      setEnabled(next.enabled)
      setEnvText(
        bound
          ? bindCodegProviderEnv(draft.envText, bound).envText
          : draft.envText
      )
      setModelProviderId(next.model_provider_id ?? null)
      setSystemPrompt(draft.systemPrompt)
      setCompactPrompt(draft.compactPrompt)
      if (next.model_provider_id != null) {
        setSelectedProviderId(next.model_provider_id)
      }
    },
    []
  )

  const load = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      const [agents, rows, preflightResult] = await Promise.all([
        acpListAgents(),
        listModelProviders(),
        acpPreflight("codeg_agent").catch(() => null),
      ])
      const next = agents.find((row) => row.agent_type === "codeg_agent")
      if (!next) {
        setAgent(null)
        setLoadError(t("missingAgent"))
        return
      }
      applyAgent(next, rows)
      setProviders(rows)
      setPreflight(preflightResult)
      if (next.model_provider_id == null) {
        const first = modelProvidersForAgent("codeg_agent", rows)[0]
        if (first) setSelectedProviderId(first.id)
      }
    } catch (err) {
      setLoadError(t("loadFailed", { message: toErrorMessage(err) }))
    } finally {
      setLoading(false)
    }
  }, [applyAgent, t])

  useEffect(() => {
    load().catch(console.error)
  }, [load])

  const bindProvider = useCallback((provider: ModelProviderInfo | null) => {
    setEnvText((current) => bindCodegProviderEnv(current, provider).envText)
    setModelProviderId(provider?.id ?? null)
  }, [])

  const persist = useCallback(async () => {
    const env = overlayCodegPromptEnv(
      parseEnvText(envText),
      systemPrompt,
      compactPrompt
    )
    const affected = await acpUpdateAgentEnv("codeg_agent", {
      enabled,
      env,
      modelProviderId,
    })
    if (affected > 0) {
      toast.info(tAcp("toasts.affectedRunningSessions", { count: affected }))
    }
    setAgent((current) =>
      current
        ? {
            ...current,
            enabled,
            env,
            model_provider_id: modelProviderId,
          }
        : current
    )
  }, [compactPrompt, enabled, envText, modelProviderId, systemPrompt, tAcp])

  const handleSave = useCallback(() => {
    if (modelProviderId == null) {
      toast.error(tAcp("toasts.modelProviderRequired"))
      return
    }
    setSaving(true)
    persistThenRunPreflight(persist, async () => {
      const result = await acpPreflight("codeg_agent", true)
      setPreflight(result)
    })
      .then(() => {
        toast.success(tAcp("toasts.configSaved"), {
          description: tAcp("toasts.configSavedHint"),
        })
      })
      .catch((err) => {
        toast.error(tAcp("toasts.saveConfigManagementFailed"), {
          description: toErrorMessage(err),
        })
      })
      .finally(() => setSaving(false))
  }, [modelProviderId, persist, tAcp])

  const failedChecks = (preflight?.checks ?? []).filter(
    (check) => check.status === "fail"
  )

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
        <Loader2 className="mr-2 h-4 w-4 animate-spin" />
        {t("loading")}
      </div>
    )
  }

  return (
    <ScrollArea className="h-full">
      <div className="space-y-6 px-3 py-3 md:px-4 md:py-4">
        <header className="space-y-1">
          <h1 className="text-sm font-semibold">{t("sectionTitle")}</h1>
          <p className="line-clamp-2 max-w-3xl text-sm leading-5 text-muted-foreground">
            {t("sectionDescription")}
          </p>
        </header>

        {loadError ? <SettingsError>{loadError}</SettingsError> : null}
        {failedChecks.length > 0 ? (
          <SettingsError>
            {`${t("preflightFailed")}: ${failedChecks
              .map((check) => `${check.label}: ${check.message}`)
              .join(" · ")}`}
          </SettingsError>
        ) : null}

        <CodegAgentProviderManager
          providers={bindableProviders}
          boundProviderId={modelProviderId}
          selectedProviderId={selectedProviderId}
          onSelectProvider={setSelectedProviderId}
          onBindProvider={bindProvider}
          bindSwitchId={CODEG_BIND_SELECT_ID}
          windowInputId={CODEG_WINDOW_INPUT_ID}
          onProvidersChanged={(rows, touched) => {
            setProviders(rows)
            if (touched) {
              setSelectedProviderId(touched.id)
              if (modelProviderId === touched.id) {
                bindProvider(touched)
              }
            }
          }}
        />

        <SettingsSection
          icon={MessageSquareText}
          title={t("promptsTitle")}
          description={t("promptsDescription")}
        >
          <CodegAgentPromptEditors
            systemPrompt={systemPrompt}
            compactPrompt={compactPrompt}
            onSystemPromptChange={setSystemPrompt}
            onCompactPromptChange={setCompactPrompt}
          />
        </SettingsSection>

        <SettingsSection
          icon={SlidersHorizontal}
          title={t("compressionTitle")}
          description={t("compressionDescription")}
        >
          <SettingCard>
            <CodegAgentCompactModelField
              envText={envText}
              onEnvTextChange={setEnvText}
              boundProvider={boundProvider}
            />
            <SettingRow
              title={t("compactSoftPercent")}
              description={t("compactSoftPercentHint")}
            >
              <Input
                type="number"
                min={1}
                max={100}
                value={codegEnvInt(
                  envText,
                  CODEG_COMPACT_SOFT_PERCENT_KEY,
                  CODEG_DEFAULT_COMPACT_SOFT_PERCENT
                )}
                onChange={(event) =>
                  setEnvText(
                    patchCodegEnvInt(
                      envText,
                      CODEG_COMPACT_SOFT_PERCENT_KEY,
                      event.target.value,
                      CODEG_DEFAULT_COMPACT_SOFT_PERCENT
                    )
                  )
                }
              />
            </SettingRow>
            <SettingRow
              title={t("compactRecentTurns")}
              description={t("compactRecentTurnsHint")}
            >
              <Input
                type="number"
                min={1}
                value={codegEnvInt(
                  envText,
                  CODEG_COMPACT_RECENT_TURNS_KEY,
                  CODEG_DEFAULT_COMPACT_RECENT_TURNS
                )}
                onChange={(event) =>
                  setEnvText(
                    patchCodegEnvInt(
                      envText,
                      CODEG_COMPACT_RECENT_TURNS_KEY,
                      event.target.value,
                      CODEG_DEFAULT_COMPACT_RECENT_TURNS
                    )
                  )
                }
              />
            </SettingRow>
          </SettingCard>
        </SettingsSection>

        <SettingsSection
          icon={Cpu}
          title={t("runtimeTitle")}
          description={t("runtimeDescription")}
        >
          <SettingCard>
            <SettingRow
              title={t("enabled")}
              description={t("enabledHint")}
              control={
                <Switch
                  checked={enabled}
                  onCheckedChange={setEnabled}
                  aria-label={t("enabled")}
                />
              }
            />
            <SettingRow title={t("maxTurns")} description={t("maxTurnsHint")}>
              <Input
                type="number"
                min={1}
                value={codegEnvInt(
                  envText,
                  CODEG_MAX_TURNS_KEY,
                  CODEG_DEFAULT_MAX_TURNS
                )}
                onChange={(event) =>
                  setEnvText(
                    patchCodegEnvInt(
                      envText,
                      CODEG_MAX_TURNS_KEY,
                      event.target.value,
                      CODEG_DEFAULT_MAX_TURNS
                    )
                  )
                }
              />
            </SettingRow>
            <SettingRow title={tAgent("maxOutput")}>
              <Input
                type="number"
                min={1}
                value={codegMaxOutputTokens(envText)}
                onChange={(event) =>
                  setEnvText(
                    patchCodegMaxOutputTokens(envText, event.target.value)
                  )
                }
              />
            </SettingRow>
          </SettingCard>
        </SettingsSection>

        <SettingsSaveBar
          onSave={handleSave}
          saving={saving}
          disabled={!agent}
          label={
            <>
              <Save className="h-3.5 w-3.5" />
              {tAgent("save")}
            </>
          }
          savingLabel={tAcp("actions.saving")}
        />
      </div>
    </ScrollArea>
  )
}
