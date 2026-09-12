"use client"

import { useCallback, useEffect, useState } from "react"
import { Wrench, Loader2 } from "lucide-react"
import { useTranslations } from "next-intl"

import { SettingCard, SettingRow } from "@/components/shared/setting-card"
import {
  SettingsError,
  SettingsSaveBar,
  SettingsSection,
} from "@/components/shared/settings-section"
import { Badge } from "@/components/ui/badge"
import { Input } from "@/components/ui/input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Switch } from "@/components/ui/switch"
import {
  getCodeIntelStatus,
  setCodeIntelSettings,
  type CodeIntelConfig,
  type CodeIntelMcpToolStatus,
  type CodeIntelStatus,
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

  const tools: CodeIntelMcpToolStatus[] = status?.mcp_tools ?? []
  const lspTools = tools.filter((tool) => tool.group === "lsp")
  const graphTools = tools.filter((tool) => tool.group === "codegraph")

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
          icon={Wrench}
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
                  title={t("lspTitle")}
                  description={t("lspAutoAttachDescription")}
                  htmlFor="code-intel-lsp-auto-attach"
                  control={
                    <Switch
                      id="code-intel-lsp-auto-attach"
                      checked={draft.lsp.auto_attach}
                      disabled={!draft.enabled}
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
                {lspTools.map((tool) => (
                  <SettingRow
                    key={tool.name}
                    title={tool.name}
                    description={tool.description}
                  >
                    <Badge
                      variant={
                        draft.enabled && draft.lsp.auto_attach
                          ? "secondary"
                          : "outline"
                      }
                    >
                      {draft.enabled && draft.lsp.auto_attach
                        ? t("mcpAdvertised")
                        : t("mcpHidden")}
                    </Badge>
                  </SettingRow>
                ))}
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
                </SettingRow>
                {graphTools.map((tool) => (
                  <SettingRow
                    key={tool.name}
                    title={tool.name}
                    description={tool.description}
                  >
                    <Badge
                      variant={
                        draft.enabled && draft.codegraph.enabled
                          ? "secondary"
                          : "outline"
                      }
                    >
                      {draft.enabled && draft.codegraph.enabled
                        ? t("mcpAdvertised")
                        : t("mcpHidden")}
                    </Badge>
                  </SettingRow>
                ))}
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
                  title={t("maxConcurrent")}
                  description={t("maxConcurrentHint")}
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
            </>
          )}
        </SettingsSection>
        <SettingsSaveBar
          onSave={() => void save()}
          saving={saving}
          disabled={!dirty}
          label={t("save")}
          savingLabel={t("saving")}
        />
      </div>
    </ScrollArea>
  )
}
