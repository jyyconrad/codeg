/**
 * 个人 Wiki 设置页，编辑捕获范围、存储目录、模型绑定、阶段技能和整理日程。
 * Wiki 专用 API 读写配置；模型和文件夹选项来自通用接口，表单转换只提交可持久化字段。
 */
"use client"

import { useCallback, useEffect, useState } from "react"
import Link from "next/link"
import { useTranslations } from "next-intl"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import { SettingCard, SettingRow } from "@/components/shared/setting-card"
import {
  SettingsError,
  SettingsSaveBar,
} from "@/components/shared/settings-section"
import { listModelProviders, listAllFolderDetails } from "@/lib/api"
import { getWikiSettings, updateWikiSettings } from "@/lib/wiki-api"
import { catalogFromProviderModel } from "@/lib/codeg-agent-catalog"
import { modelProvidersForAgent } from "@/lib/codeg-agent-providers"
import {
  completionsModelIdFromProvider,
  AGENT_LABELS,
  type ModelProviderInfo,
  type FolderDetail,
} from "@/lib/types"
import {
  dailyCronTime,
  cronForDailyTime,
  normalizeWikiSettings,
  wikiSettingsPayload,
  effectiveWikiPrompt,
  type WikiSettingsView,
  type WikiModelPromptSettings,
} from "@/lib/wiki-types"
import { toErrorMessage } from "@/lib/app-error"

function providerModels(
  provider: ModelProviderInfo | null
): { id: string; name: string }[] {
  if (!provider) return []
  const catalog = catalogFromProviderModel(provider.model)
  if (catalog)
    return catalog.models.map((model) => ({
      id: model.id,
      name: model.name || model.id,
    }))
  const model = completionsModelIdFromProvider(provider)
  return model ? [{ id: model, name: model }] : []
}
const slots = ["turn_summary", "session_rollup", "synthesize"] as const
const NONE = "__none__"
const sectionKey = {
  turn_summary: "turnSummary",
  session_rollup: "sessionRollup",
  synthesize: "synthesize",
} as const

function ModelChoice({
  id,
  slot,
  providers,
  onChange,
}: {
  id: string
  slot: WikiModelPromptSettings
  providers: ModelProviderInfo[]
  onChange: (patch: Partial<WikiModelPromptSettings>) => void
}) {
  const t = useTranslations("Wiki.v2")
  const provider =
    providers.find((item) => item.id === slot.provider_id) ?? null
  const choices = providerModels(provider)
  return (
    <div className="grid gap-3 sm:grid-cols-2">
      <label className="space-y-1 text-sm">
        <span className="text-muted-foreground">{t("provider")}</span>
        <Select
          value={slot.provider_id != null ? String(slot.provider_id) : NONE}
          onValueChange={(value) => {
            if (value === NONE) {
              onChange({ provider_id: null, model_id: null })
              return
            }
            const next = providers.find((item) => item.id === Number(value))
            onChange({
              provider_id: next?.id ?? null,
              model_id: providerModels(next ?? null)[0]?.id ?? null,
            })
          }}
        >
          <SelectTrigger id={`${id}-provider`} className="w-full">
            <SelectValue placeholder={t("chooseProvider")} />
          </SelectTrigger>
          <SelectContent align="start">
            <SelectItem value={NONE}>{t("chooseProvider")}</SelectItem>
            {slot.provider_id != null && !provider && (
              <SelectItem value={String(slot.provider_id)}>
                {t("savedProviderUnavailable")}
              </SelectItem>
            )}
            {providers.map((item) => (
              <SelectItem key={item.id} value={String(item.id)}>
                {item.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </label>
      <label className="space-y-1 text-sm">
        <span className="text-muted-foreground">{t("model")}</span>
        <Select
          value={slot.model_id ?? NONE}
          disabled={!provider}
          onValueChange={(value) =>
            onChange({ model_id: value === NONE ? null : value })
          }
        >
          <SelectTrigger id={id} className="w-full">
            <SelectValue placeholder={t("chooseModel")} />
          </SelectTrigger>
          <SelectContent align="start">
            <SelectItem value={NONE}>{t("chooseModel")}</SelectItem>
            {slot.model_id &&
              !choices.some((item) => item.id === slot.model_id) && (
                <SelectItem value={slot.model_id}>
                  {slot.model_id} · {t("unavailable")}
                </SelectItem>
              )}
            {choices.map((item) => (
              <SelectItem key={item.id} value={item.id}>
                {item.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </label>
    </div>
  )
}

export function WikiSettings() {
  const t = useTranslations("Wiki.v2")
  const old = useTranslations("WikiSettings")
  const [draft, setDraft] = useState<WikiSettingsView>(() =>
    normalizeWikiSettings(null)
  )
  const [saved, setSaved] = useState<WikiSettingsView>(() =>
    normalizeWikiSettings(null)
  )
  const [providers, setProviders] = useState<ModelProviderInfo[]>([])
  const [folders, setFolders] = useState<FolderDetail[]>([])
  const [loading, setLoading] = useState(true)
  const [loaded, setLoaded] = useState(false)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  const load = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      const [settings, providerRows, folderRows] = await Promise.all([
        getWikiSettings(),
        listModelProviders(),
        listAllFolderDetails(),
      ])
      // Suggestions come only from the first-use server response. A saved binding is never silently repaired.
      const normalized = normalizeWikiSettings(settings)
      setLoaded(true)
      setDraft(normalized)
      setSaved(normalized)
      setProviders(modelProvidersForAgent("codeg_agent", providerRows))
      setFolders(folderRows.filter((folder) => folder.kind === "regular"))
    } catch (err) {
      setError(toErrorMessage(err))
    } finally {
      setLoading(false)
    }
  }, [])
  useEffect(() => {
    void load()
  }, [load])
  const dirty =
    JSON.stringify(wikiSettingsPayload(draft)) !==
    JSON.stringify(wikiSettingsPayload(saved))
  const invalidModels = slots.some((slot) => {
    if (slot === "synthesize" && !draft.synthesize.enabled) return false
    const value = draft[slot]
    return !providerModels(
      providers.find((item) => item.id === value.provider_id) ?? null
    ).some((model) => model.id === value.model_id)
  })
  const handleSave = async () => {
    setSaving(true)
    setError(null)
    setNotice(null)
    try {
      const next = normalizeWikiSettings(
        await updateWikiSettings(wikiSettingsPayload(draft))
      )
      setDraft(next)
      setSaved(next)
      setNotice(old("saved"))
    } catch (err) {
      setError(toErrorMessage(err))
    } finally {
      setSaving(false)
    }
  }
  const changeAllModels = (patch: Partial<WikiModelPromptSettings>) =>
    setDraft((current) => ({
      ...current,
      turn_summary: { ...current.turn_summary, ...patch },
      session_rollup: { ...current.session_rollup, ...patch },
      synthesize: { ...current.synthesize, ...patch },
    }))
  const dailyTime = dailyCronTime(draft.compile_cron)
  if (loading)
    return (
      <p role="status" className="p-6 text-sm">
        {t("loading")}
      </p>
    )
  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-4xl space-y-6 p-4 md:p-6">
        <header className="space-y-2">
          <Link
            href="/?wikiView=library"
            className="text-sm text-primary underline"
          >
            {t("backToWiki")}
          </Link>
          <h1 className="text-xl font-semibold">{t("settingsTitle")}</h1>
          <p className="text-sm leading-6 text-muted-foreground">
            {t("settingsHint")}
          </p>
        </header>
        {error && (
          <SettingsError>
            {error}{" "}
            {!loaded && (
              <button
                type="button"
                className="underline"
                onClick={() => void load()}
              >
                {t("retry")}
              </button>
            )}
          </SettingsError>
        )}
        <fieldset disabled={saving} className="space-y-6">
          <SettingCard>
            <SettingRow
              title={t("enable")}
              description={t("enableHint")}
              htmlFor="wiki-enabled"
              control={
                <Switch
                  id="wiki-enabled"
                  checked={draft.enabled}
                  onCheckedChange={(enabled) =>
                    setDraft((current) => ({ ...current, enabled }))
                  }
                />
              }
            />
            <SettingRow
              title={t("autoOrganize")}
              description={t("autoOrganizeHint")}
              htmlFor="wiki-auto"
              control={
                <Switch
                  id="wiki-auto"
                  checked={draft.synthesize.enabled}
                  onCheckedChange={(enabled) =>
                    setDraft((current) => ({
                      ...current,
                      synthesize: { ...current.synthesize, enabled },
                    }))
                  }
                />
              }
            />
            <SettingRow
              title={t("dailyTime")}
              description={t("dailyTimeHint", { timezone: draft.timezone })}
              htmlFor="wiki-daily-time"
            >
              <Input
                id="wiki-daily-time"
                type="time"
                className="w-40"
                value={dailyTime ?? ""}
                onChange={(event) => {
                  const cron = cronForDailyTime(event.target.value)
                  if (cron)
                    setDraft((current) => ({ ...current, compile_cron: cron }))
                }}
              />
              {dailyTime === null && (
                <p className="mt-2 text-sm text-muted-foreground">
                  {t("customSchedule")}
                </p>
              )}
              {draft.next_compile_at && (
                <p className="mt-2 text-xs text-muted-foreground">
                  {t("nextRun", {
                    time: formatTime(draft.next_compile_at, draft.timezone),
                  })}
                </p>
              )}
            </SettingRow>
            <SettingRow
              title={t("organizingModel")}
              description={t("modelBindingHint")}
            >
              <ModelChoice
                id="wiki-main-model"
                slot={draft.synthesize}
                providers={providers}
                onChange={changeAllModels}
              />
              {invalidModels && (
                <div className="mt-3 text-sm text-amber-700 dark:text-amber-400">
                  <p>{t("modelUnavailable")}</p>
                  <Link
                    className="underline"
                    href="/settings/model-providers?from=wiki"
                  >
                    {t("checkModel")}
                  </Link>
                </div>
              )}
            </SettingRow>
          </SettingCard>
          <details className="rounded-xl border p-4">
            <summary className="cursor-pointer text-sm font-semibold">
              {t("advancedSettings")}
            </summary>
            <div className="mt-5 space-y-5">
              <SettingCard>
                <SettingRow
                  title={t("storageLocation")}
                  description={t("storageHint")}
                  htmlFor="wiki-vault-path"
                >
                  <Input
                    id="wiki-vault-path"
                    value={draft.vault_path ?? ""}
                    placeholder={
                      draft.resolved_vault_path || old("vaultPathPlaceholder")
                    }
                    onChange={(event) =>
                      setDraft((current) => ({
                        ...current,
                        vault_path: event.target.value || null,
                      }))
                    }
                  />
                </SettingRow>
                <SettingRow title={old("timezone")} htmlFor="wiki-timezone">
                  <Input
                    id="wiki-timezone"
                    value={draft.timezone}
                    onChange={(event) =>
                      setDraft((current) => ({
                        ...current,
                        timezone: event.target.value,
                      }))
                    }
                  />
                </SettingRow>
                <SettingRow title={old("compileCron")} htmlFor="wiki-cron">
                  <Input
                    id="wiki-cron"
                    value={draft.compile_cron}
                    onChange={(event) =>
                      setDraft((current) => ({
                        ...current,
                        compile_cron: event.target.value,
                      }))
                    }
                  />
                </SettingRow>
              </SettingCard>
              <SettingCard>
                <SettingRow
                  title={t("capture")}
                  description={t("captureHint")}
                  htmlFor="wiki-capture"
                  control={
                    <Switch
                      id="wiki-capture"
                      checked={draft.capture.acp_enabled}
                      onCheckedChange={(acp_enabled) =>
                        setDraft((current) => ({
                          ...current,
                          capture: { ...current.capture, acp_enabled },
                        }))
                      }
                    />
                  }
                />
                <SettingRow title={old("excludeAgentTypes")}>
                  <div className="grid gap-2 sm:grid-cols-2">
                    {Object.entries(AGENT_LABELS).map(([key, label]) => (
                      <label
                        key={key}
                        className="flex items-center gap-2 text-sm"
                      >
                        <input
                          type="checkbox"
                          checked={draft.capture.exclude_agent_types.includes(
                            key
                          )}
                          onChange={(event) =>
                            setDraft((current) => ({
                              ...current,
                              capture: {
                                ...current.capture,
                                exclude_agent_types: event.target.checked
                                  ? [
                                      ...current.capture.exclude_agent_types,
                                      key,
                                    ]
                                  : current.capture.exclude_agent_types.filter(
                                      (item) => item !== key
                                    ),
                              },
                            }))
                          }
                        />
                        {label}
                      </label>
                    ))}
                  </div>
                </SettingRow>
                <SettingRow title={old("excludeFolders")}>
                  <div className="grid gap-2 sm:grid-cols-2">
                    {folders.map((folder) => (
                      <label
                        key={folder.id}
                        className="flex items-center gap-2 text-sm"
                      >
                        <input
                          type="checkbox"
                          checked={draft.capture.exclude_folder_ids.includes(
                            folder.id
                          )}
                          onChange={(event) =>
                            setDraft((current) => ({
                              ...current,
                              capture: {
                                ...current.capture,
                                exclude_folder_ids: event.target.checked
                                  ? [
                                      ...current.capture.exclude_folder_ids,
                                      folder.id,
                                    ]
                                  : current.capture.exclude_folder_ids.filter(
                                      (id) => id !== folder.id
                                    ),
                              },
                            }))
                          }
                        />
                        {folder.alias || folder.name}
                      </label>
                    ))}
                  </div>
                </SettingRow>
              </SettingCard>
              {slots.map((slot) => (
                <section
                  key={slot}
                  aria-labelledby={`wiki-${slot}-heading`}
                  className="space-y-3"
                >
                  <h2
                    id={`wiki-${slot}-heading`}
                    className="text-sm font-semibold"
                  >
                    {t(`stages.${slot}`)}
                  </h2>
                  <ModelChoice
                    id={`wiki-${slot}-model`}
                    slot={draft[slot]}
                    providers={providers}
                    onChange={(patch) =>
                      setDraft((current) => ({
                        ...current,
                        [slot]: { ...current[slot], ...patch },
                      }))
                    }
                  />
                  <label className="block space-y-2 text-sm">
                    <span>{old(`${sectionKey[slot]}Prompt`)}</span>
                    <Textarea
                      aria-describedby={`wiki-${slot}-skill-hint`}
                      className="max-h-96 min-h-48 font-mono text-xs"
                      value={effectiveWikiPrompt(
                        draft[slot].prompt,
                        draft[`${slot}_builtin_prompt`]
                      )}
                      onChange={(event) =>
                        setDraft((current) => ({
                          ...current,
                          [slot]: {
                            ...current[slot],
                            prompt: event.target.value,
                          },
                        }))
                      }
                    />
                  </label>
                  <p
                    id={`wiki-${slot}-skill-hint`}
                    className="text-xs leading-5 text-muted-foreground"
                  >
                    {t("skillHint")}
                  </p>
                  <p className="text-xs leading-5 text-muted-foreground">
                    {t("promptBoundary")}
                  </p>
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    onClick={() =>
                      setDraft((current) => ({
                        ...current,
                        [slot]: { ...current[slot], prompt: null },
                      }))
                    }
                  >
                    {old("restoreBuiltinPrompt")}
                  </Button>
                  {draft[`${slot}_builtin_task`] && (
                    <details className="rounded-xl border p-3">
                      <summary className="cursor-pointer text-sm font-medium">
                        {t("builtinTask")}
                      </summary>
                      <div className="mt-3 space-y-2">
                        <p
                          id={`wiki-${slot}-task-hint`}
                          className="text-xs leading-5 text-muted-foreground"
                        >
                          {t("builtinTaskHint")}
                        </p>
                        <Textarea
                          aria-label={t("builtinTask")}
                          aria-describedby={`wiki-${slot}-task-hint`}
                          className="max-h-96 min-h-48 font-mono text-xs"
                          readOnly
                          value={draft[`${slot}_builtin_task`]}
                        />
                      </div>
                    </details>
                  )}
                </section>
              ))}
            </div>
          </details>
        </fieldset>
        {notice && (
          <p role="status" className="text-sm">
            {notice}
          </p>
        )}
        <div className="sticky bottom-0 flex items-center justify-end gap-3 border-t bg-background py-3">
          {dirty && (
            <>
              <p className="me-auto text-sm text-muted-foreground">
                {t("unsaved")}
              </p>
              <Button
                variant="ghost"
                size="sm"
                disabled={saving}
                onClick={() => {
                  setDraft(saved)
                  setError(null)
                  setNotice(null)
                }}
              >
                {t("discard")}
              </Button>
            </>
          )}
          <SettingsSaveBar
            onSave={() => void handleSave()}
            saving={saving}
            disabled={!loaded || !dirty || (draft.enabled && invalidModels)}
            label={old("save")}
            savingLabel={old("saving")}
          />
        </div>
      </div>
    </div>
  )
}
function formatTime(value: string, timezone: string): string {
  try {
    return new Intl.DateTimeFormat(undefined, {
      dateStyle: "medium",
      timeStyle: "short",
      timeZone: timezone,
    }).format(new Date(value))
  } catch {
    return value
  }
}
