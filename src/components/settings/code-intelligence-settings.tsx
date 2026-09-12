"use client"

import { useCallback, useEffect, useMemo, useState } from "react"
import { Braces, Loader2, Plus } from "lucide-react"
import { useTranslations } from "next-intl"

import { SettingCard, SettingRow } from "@/components/shared/setting-card"
import {
  SettingsError,
  SettingsSaveBar,
  SettingsSection,
} from "@/components/shared/settings-section"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Switch } from "@/components/ui/switch"
import {
  getCodeIntelStatus,
  setCodeIntelSettings,
  type CodeIntelConfig,
  type CodeIntelStatus,
  type CustomLspServer,
  type LspServerStatus,
} from "@/lib/api"
import { toErrorMessage } from "@/lib/app-error"
import { useAppWorkspaceStore } from "@/stores/app-workspace-store"

const MAX_CONCURRENT_MIN = 1
const MAX_CONCURRENT_MAX = 8
const DEFAULT_MAX_CONCURRENT = 2

function cloneConfig(config: CodeIntelConfig): CodeIntelConfig {
  return structuredClone(config)
}

function clampConcurrent(n: number): number {
  if (!Number.isFinite(n)) return DEFAULT_MAX_CONCURRENT
  return Math.min(
    MAX_CONCURRENT_MAX,
    Math.max(MAX_CONCURRENT_MIN, Math.trunc(n))
  )
}

function parseExtensions(raw: string): string[] {
  return raw
    .split(",")
    .map((part) => part.trim())
    .filter(Boolean)
}

function customFromDraft(
  server: CustomLspServer,
  checked: string[]
): LspServerStatus {
  return {
    id: server.id,
    language: server.language,
    binary: server.command,
    binary_on_path: false,
    checked: checked.includes(server.id),
    language_detected: false,
    default_checked: false,
    custom: true,
  }
}

export function CodeIntelligenceSettings() {
  const t = useTranslations("CodeIntelligenceSettings")
  const activeFolder = useAppWorkspaceStore((s) => {
    const id = s.activeFolderId
    return id == null ? null : (s.allFolders.find((f) => f.id === id) ?? null)
  })
  const cwd = activeFolder?.path ?? null

  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [status, setStatus] = useState<CodeIntelStatus | null>(null)
  const [draft, setDraft] = useState<CodeIntelConfig | null>(null)
  const [baseline, setBaseline] = useState<CodeIntelConfig | null>(null)
  const [customId, setCustomId] = useState("")
  const [customLanguage, setCustomLanguage] = useState("")
  const [customCommand, setCustomCommand] = useState("")
  const [customExtensions, setCustomExtensions] = useState("")

  const applyStatus = useCallback((next: CodeIntelStatus) => {
    setStatus(next)
    setDraft(cloneConfig(next.config))
    setBaseline(cloneConfig(next.config))
    setLoadError(null)
    setSaveError(null)
  }, [])

  const load = useCallback(async () => {
    setLoading(true)
    setLoadError(null)
    try {
      applyStatus(await getCodeIntelStatus(cwd))
    } catch (err) {
      setLoadError(toErrorMessage(err))
    } finally {
      setLoading(false)
    }
  }, [applyStatus, cwd])

  useEffect(() => {
    void load()
  }, [load])

  const dirty =
    draft != null &&
    baseline != null &&
    JSON.stringify(draft) !== JSON.stringify(baseline)

  const rows = useMemo(() => {
    if (!draft || !status) return []
    const fromStatus = status.lsp_servers
    const extras = draft.lsp.custom.filter(
      (server) => !fromStatus.some((row) => row.id === server.id)
    )
    return [
      ...fromStatus,
      ...extras.map((server) => customFromDraft(server, draft.lsp.checked)),
    ]
  }, [draft, status])

  const save = useCallback(async () => {
    if (!draft) return
    const payload: CodeIntelConfig = {
      ...draft,
      codegraph: {
        ...draft.codegraph,
        binary_path: draft.codegraph.binary_path?.trim() || null,
      },
      lsp: {
        ...draft.lsp,
        max_concurrent: clampConcurrent(draft.lsp.max_concurrent),
      },
    }
    setSaving(true)
    setSaveError(null)
    try {
      await setCodeIntelSettings(payload)
      applyStatus(await getCodeIntelStatus(cwd))
    } catch (err) {
      setSaveError(toErrorMessage(err))
    } finally {
      setSaving(false)
    }
  }, [applyStatus, cwd, draft])

  const addCustom = useCallback(() => {
    setDraft((prev) => {
      if (!prev) return prev
      const server: CustomLspServer = {
        id: customId.trim() || `custom-${prev.lsp.custom.length + 1}`,
        language: customLanguage.trim(),
        command: customCommand.trim(),
        args: [],
        extensions: parseExtensions(customExtensions),
        manifests: [],
      }
      return {
        ...prev,
        lsp: {
          ...prev.lsp,
          custom: [...prev.lsp.custom, server],
        },
      }
    })
    setCustomId("")
    setCustomLanguage("")
    setCustomCommand("")
    setCustomExtensions("")
  }, [customCommand, customExtensions, customId, customLanguage])

  const toggleChecked = useCallback((id: string, checked: boolean) => {
    setDraft((prev) => {
      if (!prev) return prev
      const nextChecked = checked
        ? prev.lsp.checked.includes(id)
          ? prev.lsp.checked
          : [...prev.lsp.checked, id]
        : prev.lsp.checked.filter((item) => item !== id)
      return {
        ...prev,
        lsp: { ...prev.lsp, checked: nextChecked },
      }
    })
  }, [])

  if (loading && !draft) {
    return (
      <div className="flex h-full items-center justify-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" />
        {t("loading")}
      </div>
    )
  }

  return (
    <ScrollArea className="h-full">
      <div className="w-full space-y-4 p-3 md:p-4">
        <SettingsSection
          icon={Braces}
          title={t("sectionTitle")}
          description={t("sectionDescription")}
        >
          {loadError && (
            <SettingsError>
              {t("loadFailed", { message: loadError })}
            </SettingsError>
          )}
          {saveError && (
            <SettingsError>
              {t("saveFailed", { message: saveError })}
            </SettingsError>
          )}

          {draft && (
            <>
              <SettingCard>
                <SettingRow
                  title={t("masterTitle")}
                  description={t("masterDescription")}
                  htmlFor="code-intel-enabled"
                  control={
                    <Switch
                      id="code-intel-enabled"
                      checked={draft.enabled}
                      onCheckedChange={(enabled) =>
                        setDraft({ ...draft, enabled })
                      }
                    />
                  }
                />
              </SettingCard>

              <SettingCard>
                <SettingRow
                  title={t("codegraphTitle")}
                  description={t("codegraphDescription")}
                  htmlFor="code-intel-codegraph"
                  control={
                    <Switch
                      id="code-intel-codegraph"
                      checked={draft.codegraph.enabled}
                      disabled={!draft.enabled}
                      onCheckedChange={(enabled) =>
                        setDraft({
                          ...draft,
                          codegraph: {
                            ...draft.codegraph,
                            enabled,
                          },
                        })
                      }
                    />
                  }
                >
                  <p className="text-xs text-muted-foreground">
                    {status?.codegraph_binary
                      ? t("codegraphOnPath", {
                          path: status.codegraph_binary,
                        })
                      : t("codegraphMissing")}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {status?.codegraph_indexed
                      ? t("codegraphIndexed")
                      : t("codegraphNotIndexed")}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {t("codegraphInstall")}
                  </p>
                </SettingRow>
                <SettingRow
                  title={t("binaryPath")}
                  htmlFor="code-intel-binary-path"
                >
                  <Input
                    id="code-intel-binary-path"
                    value={draft.codegraph.binary_path ?? ""}
                    onChange={(event) => {
                      const value = event.target.value
                      setDraft({
                        ...draft,
                        codegraph: {
                          ...draft.codegraph,
                          binary_path: value.trim() ? value : null,
                        },
                      })
                    }}
                  />
                </SettingRow>
              </SettingCard>

              <SettingCard>
                <SettingRow
                  title={t("lspTitle")}
                  description={t("lspAutoAttachDescription", {
                    n: clampConcurrent(draft.lsp.max_concurrent),
                  })}
                />
                <SettingRow
                  title={t("lspAutoAttach")}
                  htmlFor="code-intel-lsp-auto-attach"
                  control={
                    <Switch
                      id="code-intel-lsp-auto-attach"
                      checked={draft.lsp.auto_attach}
                      onCheckedChange={(autoAttach) =>
                        setDraft({
                          ...draft,
                          lsp: {
                            ...draft.lsp,
                            auto_attach: autoAttach,
                          },
                        })
                      }
                    />
                  }
                />
                <SettingRow
                  title={t("maxConcurrent")}
                  htmlFor="code-intel-max-concurrent"
                  control={
                    <Input
                      id="code-intel-max-concurrent"
                      type="number"
                      min={MAX_CONCURRENT_MIN}
                      max={MAX_CONCURRENT_MAX}
                      value={
                        Number.isFinite(draft.lsp.max_concurrent)
                          ? draft.lsp.max_concurrent
                          : ""
                      }
                      onChange={(event) => {
                        const raw = event.target.value
                        setDraft({
                          ...draft,
                          lsp: {
                            ...draft.lsp,
                            max_concurrent:
                              raw === "" ? Number.NaN : Number(raw),
                          },
                        })
                      }}
                      className="h-8 w-24 bg-background text-xs"
                    />
                  }
                />
              </SettingCard>

              <SettingCard>
                {rows.map((server, index) => {
                  const checkId = `code-intel-lsp-${server.id || index}`
                  const checked = draft.lsp.checked.includes(server.id)
                  return (
                    <SettingRow
                      key={`${server.id}-${index}`}
                      title={server.id || server.language}
                      description={server.language}
                      htmlFor={checkId}
                      control={
                        <Checkbox
                          id={checkId}
                          checked={checked}
                          aria-label={server.id || server.language}
                          onCheckedChange={(value) =>
                            toggleChecked(server.id, value === true)
                          }
                        />
                      }
                    >
                      <div className="flex flex-wrap items-center gap-1.5">
                        {server.binary ? (
                          <span className="text-xs text-muted-foreground">
                            {server.binary}
                          </span>
                        ) : null}
                        <Badge
                          variant={
                            server.binary_on_path ? "secondary" : "outline"
                          }
                        >
                          {server.binary_on_path ? t("onPath") : t("notOnPath")}
                        </Badge>
                        <Badge
                          variant={
                            server.language_detected ? "secondary" : "outline"
                          }
                        >
                          {server.language_detected
                            ? t("detected")
                            : t("notDetected")}
                        </Badge>
                      </div>
                    </SettingRow>
                  )
                })}
              </SettingCard>

              <SettingCard>
                <SettingRow title={t("customTitle")}>
                  <div className="flex flex-col gap-2 sm:flex-row">
                    <Input
                      value={customId}
                      onChange={(event) => setCustomId(event.target.value)}
                      placeholder="id"
                      aria-label="id"
                    />
                    <Input
                      value={customLanguage}
                      onChange={(event) =>
                        setCustomLanguage(event.target.value)
                      }
                      placeholder="language"
                      aria-label="language"
                    />
                    <Input
                      value={customCommand}
                      onChange={(event) => setCustomCommand(event.target.value)}
                      placeholder="command"
                      aria-label="command"
                    />
                    <Input
                      value={customExtensions}
                      onChange={(event) =>
                        setCustomExtensions(event.target.value)
                      }
                      placeholder="ext,ext"
                      aria-label="extensions"
                    />
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      onClick={addCustom}
                    >
                      <Plus className="h-3.5 w-3.5" />
                      {t("addCustom")}
                    </Button>
                  </div>
                </SettingRow>
              </SettingCard>

              <SettingsSaveBar
                onSave={() => void save()}
                saving={saving}
                disabled={!dirty || loading}
                label={t("save")}
                savingLabel={t("saving")}
              />
            </>
          )}
        </SettingsSection>
      </div>
    </ScrollArea>
  )
}
