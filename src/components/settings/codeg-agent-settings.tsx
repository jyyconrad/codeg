"use client"

import { useCallback, useEffect, useMemo, useState } from "react"
import {
  Cpu,
  Loader2,
  MessageSquareText,
  Save,
  Server,
  SlidersHorizontal,
} from "lucide-react"
import { useTranslations } from "next-intl"
import { useRouter } from "next/navigation"
import { toast } from "sonner"

import {
  SettingCard,
  SettingNote,
  SettingRow,
} from "@/components/shared/setting-card"
import {
  SettingsError,
  SettingsSaveBar,
  SettingsSection,
} from "@/components/shared/settings-section"
import { AddModelProviderDialog } from "@/components/settings/add-model-provider-dialog"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ScrollArea } from "@/components/ui/scroll-area"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
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
  codegWindowForModel,
  overlayCodegPromptEnv,
  persistThenRunPreflight,
  patchCodegContextWindow,
  patchCodegEnvInt,
  patchCodegMaxOutputTokens,
} from "@/lib/codeg-agent-env"
import {
  modelProviderOptionLabel,
  modelProvidersForAgent,
} from "@/lib/codeg-agent-providers"
import { parseEnvText } from "@/lib/env-text"
import type {
  AcpAgentInfo,
  ModelProviderInfo,
  PreflightResult,
} from "@/lib/types"
import { completionsModelIdFromProvider } from "@/lib/types"

export function CodegAgentSettings() {
  const t = useTranslations("CodegAgentSettings")
  const tAgent = useTranslations("AcpAgentSettings.codegAgent")
  const tAcp = useTranslations("AcpAgentSettings")
  const router = useRouter()

  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [agent, setAgent] = useState<AcpAgentInfo | null>(null)
  const [providers, setProviders] = useState<ModelProviderInfo[]>([])
  const [addDialogOpen, setAddDialogOpen] = useState(false)
  const [preflight, setPreflight] = useState<PreflightResult | null>(null)

  const [enabled, setEnabled] = useState(false)
  const [envText, setEnvText] = useState("")
  const [modelProviderId, setModelProviderId] = useState<number | null>(null)
  const [systemPrompt, setSystemPrompt] = useState("")
  const [compactPrompt, setCompactPrompt] = useState("")

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

  const modelId = boundProvider
    ? completionsModelIdFromProvider(boundProvider)
    : ""
  const windowValue = codegWindowForModel(envText, modelId)

  const applyAgent = useCallback((next: AcpAgentInfo) => {
    const draft = codegDraftFromEnv(next.env)
    setAgent(next)
    setEnabled(next.enabled)
    setEnvText(draft.envText)
    setModelProviderId(next.model_provider_id ?? null)
    setSystemPrompt(draft.systemPrompt)
    setCompactPrompt(draft.compactPrompt)
  }, [])

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
      applyAgent(next)
      setProviders(rows)
      setPreflight(preflightResult)
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
        <SettingsSection
          title={t("sectionTitle")}
          description={t("sectionDescription")}
        >
          <SettingNote>{tAgent("inProcess")}</SettingNote>
          <SettingNote>{tAgent("protocol")}</SettingNote>
          {loadError ? <SettingsError>{loadError}</SettingsError> : null}
          {failedChecks.length > 0 ? (
            <SettingsError>
              {`${t("preflightFailed")}: ${failedChecks
                .map((check) => `${check.label}: ${check.message}`)
                .join(" · ")}`}
            </SettingsError>
          ) : null}
        </SettingsSection>

        <SettingsSection
          icon={Server}
          title={t("providerTitle")}
          description={t("providerDescription")}
        >
          <SettingCard>
            <SettingRow
              title={tAgent("bindProvider")}
              description={t("presetHint")}
              htmlFor={CODEG_BIND_SELECT_ID}
            >
              {bindableProviders.length > 0 ? (
                <Select
                  value={modelProviderId != null ? String(modelProviderId) : ""}
                  onValueChange={(value) => {
                    const next =
                      bindableProviders.find(
                        (provider) => String(provider.id) === value
                      ) ?? null
                    bindProvider(next)
                  }}
                >
                  <SelectTrigger id={CODEG_BIND_SELECT_ID} className="w-full">
                    <SelectValue placeholder={tAcp("selectModelProvider")} />
                  </SelectTrigger>
                  <SelectContent align="start">
                    {bindableProviders.map((provider) => (
                      <SelectItem key={provider.id} value={String(provider.id)}>
                        {modelProviderOptionLabel(provider, "codeg_agent")}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              ) : (
                <p className="text-xs text-muted-foreground">
                  {tAcp("noModelProviderAvailable")}
                </p>
              )}
              <div className="mt-2 flex flex-wrap gap-2">
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={() => setAddDialogOpen(true)}
                >
                  {tAgent("addModelProvider")}
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  onClick={() => router.push("/settings/model-providers")}
                >
                  {t("manageProviders")}
                </Button>
              </div>
            </SettingRow>
            <SettingRow title={tAgent("modelReadOnly")}>
              <Input
                value={modelId}
                readOnly
                placeholder={tAgent("modelSelectorHint")}
              />
            </SettingRow>
            <SettingRow
              title={tAgent("contextWindow")}
              htmlFor={CODEG_WINDOW_INPUT_ID}
            >
              <Input
                id={CODEG_WINDOW_INPUT_ID}
                type="number"
                min={1}
                value={windowValue ?? ""}
                disabled={!modelId}
                onChange={(event) => {
                  const next = Number.parseInt(event.target.value, 10)
                  if (!Number.isFinite(next) || next <= 0) return
                  setEnvText(patchCodegContextWindow(envText, modelId, next))
                }}
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
            {boundProvider ? (
              <SettingRow title={tAgent("boundCredentials")}>
                <p className="break-all text-xs text-muted-foreground">
                  {boundProvider.api_url}
                </p>
                <p className="text-xs text-muted-foreground">
                  {boundProvider.api_key_masked || boundProvider.api_key}
                </p>
              </SettingRow>
            ) : null}
          </SettingCard>
        </SettingsSection>

        <SettingsSection
          icon={MessageSquareText}
          title={t("promptsTitle")}
          description={t("promptsDescription")}
        >
          <SettingCard>
            <SettingRow
              title={tAgent("systemPrompt")}
              description={tAgent("systemPromptHint")}
            >
              <Textarea
                value={systemPrompt}
                onChange={(event) => setSystemPrompt(event.target.value)}
                placeholder={tAgent("emptyUsesBuiltin")}
                className="min-h-24"
              />
            </SettingRow>
            <SettingRow
              title={tAgent("compactPrompt")}
              description={tAgent("compactPromptHint")}
            >
              <Textarea
                value={compactPrompt}
                onChange={(event) => setCompactPrompt(event.target.value)}
                placeholder={tAgent("emptyUsesBuiltin")}
                className="min-h-24"
              />
            </SettingRow>
          </SettingCard>
        </SettingsSection>

        <SettingsSection
          icon={SlidersHorizontal}
          title={t("compressionTitle")}
          description={t("compressionDescription")}
        >
          <SettingCard>
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

      <AddModelProviderDialog
        open={addDialogOpen}
        onOpenChange={setAddDialogOpen}
        defaultAgentType="codeg_agent"
        onProviderAdded={(created) => {
          void listModelProviders().then((rows) => {
            setProviders(rows)
            bindProvider(created)
          })
        }}
      />
    </ScrollArea>
  )
}
