"use client"

import { useCallback, useEffect, useMemo, useState } from "react"
import {
  BookMarked,
  Clock,
  Folder,
  Loader2,
  Save,
  Sparkles,
  WandSparkles,
} from "lucide-react"
import { useTranslations } from "next-intl"
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
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import { toErrorMessage } from "@/lib/app-error"
import {
  getWikiSettings,
  listAllFolderDetails,
  updateWikiSettings,
} from "@/lib/api"
import { getAgentLabel } from "@/lib/custom-agents"
import {
  AGENT_LABELS,
  type BuiltinAgentType,
  type FolderDetail,
} from "@/lib/types"
import {
  normalizeWikiSettings,
  wikiSettingsPayload,
  type WikiSettingsView,
} from "@/lib/wiki-types"

const BUILTIN_AGENT_TYPES = Object.keys(AGENT_LABELS) as BuiltinAgentType[]

function parseFolderIds(value: string): number[] {
  const ids: number[] = []
  const seen = new Set<number>()
  for (const part of value.split(/[, \s]+/)) {
    if (!part) continue
    const n = Number(part)
    if (!Number.isInteger(n) || seen.has(n)) continue
    seen.add(n)
    ids.push(n)
  }
  return ids
}

function formatTimestamp(value: string | null | undefined, tz: string): string {
  if (!value) return ""
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  try {
    return new Intl.DateTimeFormat(undefined, {
      dateStyle: "medium",
      timeStyle: "short",
      timeZone: tz || undefined,
    }).format(date)
  } catch {
    return date.toLocaleString()
  }
}

export function WikiSettings() {
  const t = useTranslations("WikiSettings")

  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [folders, setFolders] = useState<FolderDetail[]>([])
  const [draft, setDraft] = useState<WikiSettingsView>(() =>
    normalizeWikiSettings(null)
  )
  const [folderIdsText, setFolderIdsText] = useState("")

  const apply = useCallback((next: WikiSettingsView) => {
    setDraft(next)
    setFolderIdsText(next.capture.exclude_folder_ids.join(", "))
  }, [])

  const load = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      const [settings, folderRows] = await Promise.all([
        getWikiSettings(),
        listAllFolderDetails().catch(() => [] as FolderDetail[]),
      ])
      apply(normalizeWikiSettings(settings))
      setFolders(folderRows.filter((folder) => folder.kind === "regular"))
    } catch (err) {
      apply(normalizeWikiSettings(null))
      setLoadError(t("loadFailed", { message: toErrorMessage(err) }))
    } finally {
      setLoading(false)
    }
  }, [apply, t])

  useEffect(() => {
    load().catch(console.error)
  }, [load])

  const excludeAgents = draft.capture.exclude_agent_types
  const extraAgents = useMemo(
    () =>
      excludeAgents.filter(
        (type) => !(BUILTIN_AGENT_TYPES as string[]).includes(type)
      ),
    [excludeAgents]
  )
  const agentTypes = useMemo(
    () => [...BUILTIN_AGENT_TYPES, ...extraAgents],
    [extraAgents]
  )

  const excludeFolderIds = draft.capture.exclude_folder_ids
  const extraFolderIds = useMemo(() => {
    const known = new Set(folders.map((folder) => folder.id))
    return excludeFolderIds.filter((id) => !known.has(id))
  }, [excludeFolderIds, folders])

  const setCapture = useCallback(
    (patch: Partial<WikiSettingsView["capture"]>) => {
      setDraft((current) => ({
        ...current,
        capture: { ...current.capture, ...patch },
      }))
    },
    []
  )

  const toggleAgent = useCallback(
    (type: string, checked: boolean) => {
      setCapture({
        exclude_agent_types: checked
          ? excludeAgents.includes(type)
            ? excludeAgents
            : [...excludeAgents, type]
          : excludeAgents.filter((item) => item !== type),
      })
    },
    [excludeAgents, setCapture]
  )

  const toggleFolder = useCallback(
    (id: number, checked: boolean) => {
      const next = checked
        ? excludeFolderIds.includes(id)
          ? excludeFolderIds
          : [...excludeFolderIds, id]
        : excludeFolderIds.filter((item) => item !== id)
      setCapture({ exclude_folder_ids: next })
      setFolderIdsText(next.join(", "))
    },
    [excludeFolderIds, setCapture]
  )

  const handleFolderIdsText = useCallback(
    (value: string) => {
      setFolderIdsText(value)
      setCapture({ exclude_folder_ids: parseFolderIds(value) })
    },
    [setCapture]
  )

  const handleSave = useCallback(() => {
    setSaving(true)
    const payload = wikiSettingsPayload({
      ...draft,
      capture: {
        ...draft.capture,
        exclude_folder_ids: parseFolderIds(folderIdsText),
      },
    })
    updateWikiSettings(payload)
      .then((saved) => {
        apply(normalizeWikiSettings(saved))
        toast.success(t("saved"))
      })
      .catch((err) => {
        toast.error(t("saveFailed", { message: toErrorMessage(err) }))
      })
      .finally(() => setSaving(false))
  }, [apply, draft, folderIdsText, t])

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
        <Loader2 className="mr-2 h-4 w-4 animate-spin" />
        {t("loading")}
      </div>
    )
  }

  const nextCompile = formatTimestamp(draft.next_compile_at, draft.timezone)

  return (
    <ScrollArea className="h-full">
      <div className="space-y-6 px-3 py-3 md:px-4 md:py-4">
        <header className="space-y-1">
          <h1 className="text-sm font-semibold">{t("sectionTitle")}</h1>
          <p className="line-clamp-3 max-w-3xl text-sm leading-5 text-muted-foreground">
            {t("sectionDescription")}
          </p>
        </header>

        {loadError ? (
          <SettingsError>
            {loadError}{" "}
            <button
              type="button"
              className="underline"
              onClick={() => load().catch(console.error)}
            >
              {t("retry")}
            </button>
          </SettingsError>
        ) : null}

        <SettingsSection
          icon={BookMarked}
          title={t("enabled")}
          description={t("enabledHint")}
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
        >
          <SettingNote>{t("enabledOffHint")}</SettingNote>
        </SettingsSection>

        {(draft.next_compile_at != null ||
          draft.pending_source_count != null) && (
          <SettingsSection
            icon={Clock}
            title={t("statusTitle")}
            description={t("statusDescription")}
          >
            <SettingCard>
              <SettingRow title={t("nextCompileAt")}>
                <p className="text-sm">{nextCompile || t("notScheduled")}</p>
              </SettingRow>
              {draft.pending_source_count != null ? (
                <SettingRow title={t("pendingSources")}>
                  <p className="text-sm">{draft.pending_source_count}</p>
                </SettingRow>
              ) : null}
            </SettingCard>
          </SettingsSection>
        )}

        <SettingsSection
          icon={Folder}
          title={t("vaultTitle")}
          description={t("vaultDescription")}
        >
          <SettingCard>
            <SettingRow
              title={t("vaultPath")}
              description={t("vaultPathHint")}
              htmlFor="wiki-vault-path"
            >
              <Input
                id="wiki-vault-path"
                value={draft.vault_path ?? ""}
                placeholder={t("vaultPathPlaceholder")}
                onChange={(event) =>
                  setDraft((current) => ({
                    ...current,
                    vault_path: event.target.value,
                  }))
                }
              />
            </SettingRow>
            <SettingRow
              title={t("timezone")}
              description={t("timezoneHint")}
              htmlFor="wiki-timezone"
            >
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
            <SettingRow
              title={t("compileCron")}
              description={t("compileCronHint")}
              htmlFor="wiki-compile-cron"
            >
              <Input
                id="wiki-compile-cron"
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
        </SettingsSection>

        <SettingsSection
          icon={Sparkles}
          title={t("captureTitle")}
          description={t("captureDescription")}
        >
          <SettingCard>
            <SettingRow
              title={t("acpEnabled")}
              description={t("acpEnabledHint")}
              htmlFor="wiki-acp-enabled"
              control={
                <Switch
                  id="wiki-acp-enabled"
                  checked={draft.capture.acp_enabled}
                  onCheckedChange={(acp_enabled) => setCapture({ acp_enabled })}
                />
              }
            />
            <SettingRow
              title={t("excludeAgentTypes")}
              description={t("excludeAgentTypesHint")}
            >
              <div className="grid gap-2 sm:grid-cols-2">
                {agentTypes.map((type) => {
                  const id = `wiki-exclude-agent-${type}`
                  return (
                    <label
                      key={type}
                      htmlFor={id}
                      className="flex items-center gap-2 text-sm"
                    >
                      <Checkbox
                        id={id}
                        checked={excludeAgents.includes(type)}
                        onCheckedChange={(value) =>
                          toggleAgent(type, value === true)
                        }
                      />
                      <span className="truncate">{getAgentLabel(type)}</span>
                    </label>
                  )
                })}
              </div>
            </SettingRow>
            <SettingRow
              title={t("excludeFolders")}
              description={t("excludeFoldersHint")}
              htmlFor="wiki-exclude-folders"
            >
              {folders.length > 0 || extraFolderIds.length > 0 ? (
                <div className="mb-2 grid gap-2 sm:grid-cols-2">
                  {folders.map((folder) => {
                    const id = `wiki-exclude-folder-${folder.id}`
                    return (
                      <label
                        key={folder.id}
                        htmlFor={id}
                        className="flex items-center gap-2 text-sm"
                      >
                        <Checkbox
                          id={id}
                          checked={excludeFolderIds.includes(folder.id)}
                          onCheckedChange={(value) =>
                            toggleFolder(folder.id, value === true)
                          }
                        />
                        <span className="truncate">
                          {folder.alias ?? folder.name}
                          <span className="text-muted-foreground">
                            {" "}
                            ({folder.id})
                          </span>
                        </span>
                      </label>
                    )
                  })}
                  {extraFolderIds.map((folderId) => {
                    const id = `wiki-exclude-folder-${folderId}`
                    return (
                      <label
                        key={folderId}
                        htmlFor={id}
                        className="flex items-center gap-2 text-sm"
                      >
                        <Checkbox
                          id={id}
                          checked={excludeFolderIds.includes(folderId)}
                          onCheckedChange={(value) =>
                            toggleFolder(folderId, value === true)
                          }
                        />
                        <span className="truncate">
                          {t("unknownFolder", { id: folderId })}
                        </span>
                      </label>
                    )
                  })}
                </div>
              ) : null}
              <Input
                id="wiki-exclude-folders"
                value={folderIdsText}
                placeholder={t("excludeFoldersPlaceholder")}
                onChange={(event) => handleFolderIdsText(event.target.value)}
              />
            </SettingRow>
          </SettingCard>
        </SettingsSection>

        <SettingsSection
          icon={WandSparkles}
          title={t("ingestTitle")}
          description={t("ingestDescription")}
        >
          <SettingCard>
            <SettingRow
              title={t("ingestModel")}
              description={t("ingestModelHint")}
              htmlFor="wiki-ingest-model"
            >
              <Input
                id="wiki-ingest-model"
                value={draft.ingest.model_id ?? ""}
                placeholder={t("ingestModelPlaceholder")}
                onChange={(event) =>
                  setDraft((current) => ({
                    ...current,
                    ingest: {
                      ...current.ingest,
                      model_id: event.target.value,
                    },
                  }))
                }
              />
            </SettingRow>
            <SettingRow title={t("ingestPrompt")} htmlFor="wiki-ingest-prompt">
              <p className="mb-2 text-xs text-muted-foreground">
                {t("ingestPromptBuiltinHint")}
              </p>
              <Textarea
                id="wiki-ingest-prompt"
                value={draft.ingest.prompt ?? ""}
                placeholder={t("ingestPromptPlaceholder")}
                className="min-h-24"
                onChange={(event) =>
                  setDraft((current) => ({
                    ...current,
                    ingest: {
                      ...current.ingest,
                      prompt: event.target.value,
                    },
                  }))
                }
              />
            </SettingRow>
          </SettingCard>
        </SettingsSection>

        <SettingsSection
          icon={Save}
          title={t("compileTitle")}
          description={t("compileDescription")}
        >
          <SettingCard>
            <SettingRow
              title={t("compileEnabled")}
              description={t("compileEnabledHint")}
              htmlFor="wiki-compile-enabled"
              control={
                <Switch
                  id="wiki-compile-enabled"
                  checked={draft.compile.enabled}
                  onCheckedChange={(enabled) =>
                    setDraft((current) => ({
                      ...current,
                      compile: { ...current.compile, enabled },
                    }))
                  }
                />
              }
            />
            {!draft.compile.enabled ? (
              <SettingRow title={t("compileNowUnavailable")} />
            ) : null}
            <SettingRow
              title={t("compileModel")}
              description={t("compileModelHint")}
              htmlFor="wiki-compile-model"
            >
              <Input
                id="wiki-compile-model"
                value={draft.compile.model_id ?? ""}
                placeholder={t("compileModelPlaceholder")}
                onChange={(event) =>
                  setDraft((current) => ({
                    ...current,
                    compile: {
                      ...current.compile,
                      model_id: event.target.value,
                    },
                  }))
                }
              />
            </SettingRow>
            <SettingRow
              title={t("compilePrompt")}
              htmlFor="wiki-compile-prompt"
            >
              <p className="mb-2 text-xs text-muted-foreground">
                {t("compilePromptBuiltinHint")}
              </p>
              <Textarea
                id="wiki-compile-prompt"
                value={draft.compile.prompt ?? ""}
                placeholder={t("compilePromptPlaceholder")}
                className="min-h-24"
                onChange={(event) =>
                  setDraft((current) => ({
                    ...current,
                    compile: {
                      ...current.compile,
                      prompt: event.target.value,
                    },
                  }))
                }
              />
            </SettingRow>
          </SettingCard>
        </SettingsSection>

        <SettingsSaveBar
          onSave={handleSave}
          saving={saving}
          label={t("save")}
          savingLabel={t("saving")}
        />
      </div>
    </ScrollArea>
  )
}
