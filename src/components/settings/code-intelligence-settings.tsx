"use client"

import { useCallback, useEffect, useState } from "react"
import { Loader2, Wrench } from "lucide-react"
import { useTranslations } from "next-intl"

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
import { Badge } from "@/components/ui/badge"
import { BrowserLink } from "@/components/ui/browser-link"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Switch } from "@/components/ui/switch"
import {
  getCodeIntelStatus,
  retryCodeIntel,
  setCodeIntelSettings,
  type CodeIntelConfig,
  type CodeIntelLanguageStatus,
  type CodeIntelProviderId,
  type CodeIntelProviderStatus,
  type CodeIntelStatus,
} from "@/lib/api"
import { toErrorMessage } from "@/lib/app-error"
import { cn } from "@/lib/utils"
import { useAppWorkspaceStore } from "@/stores/app-workspace-store"

const MAX_CONCURRENT_MIN = 1
const MAX_CONCURRENT_MAX = 8
const DEFAULT_MAX_CONCURRENT = 2
const SERENA_PIN = "v1.7.0"
const SERENA_DOCS = "https://github.com/oraios/serena"
const TOOL_IDS = ["lsp", "codegraph", "serena"] as const
const DEFAULT_SERENA_MODES = ["interactive", "editing", "planning"] as const
const SERENA_CONTEXTS = [
  "agent",
  "antigravity",
  "chatgpt",
  "claude-code",
  "codebuddy",
  "codex",
  "copilot-cli",
  "desktop-app",
  "grok",
  "ide",
  "jb-ai-assistant",
  "jb-copilot-plugin",
  "junie",
  "oaicompat-agent",
  "vscode",
] as const

const SHELL_META = /[\u0000-\u001f\u007f;&|<>`$"'*?#!(){}[\]~]/

type StatusKey =
  | "connected"
  | "stopped"
  | "starting"
  | "exited"
  | "handshakeFailed"
  | "waiting"
  | "notInstalled"
  | "unconfigured"
  | "installFailed"
  | "downloading"
  | "installed"
  | "global"
  | "managed"
  | "override"

type Translator = ReturnType<typeof useTranslations<"CodeIntelligenceSettings">>

function cloneConfig(config: CodeIntelConfig): CodeIntelConfig {
  return structuredClone(config)
}

function clampConcurrent(value: number): number {
  if (!Number.isFinite(value)) return DEFAULT_MAX_CONCURRENT
  return Math.min(
    MAX_CONCURRENT_MAX,
    Math.max(MAX_CONCURRENT_MIN, Math.trunc(value))
  )
}

function isAbsolutePath(value: string): boolean {
  return (
    value.startsWith("/") ||
    value.startsWith("\\\\") ||
    /^[A-Za-z]:[\\/]/.test(value)
  )
}

function isCommandName(value: string): boolean {
  return /^[A-Za-z0-9][A-Za-z0-9._+-]*$/.test(value)
}

function normalizeCommandSpec(
  value: string | null
): { ok: true; value: string | null } | { ok: false } {
  if (value == null) return { ok: true, value: null }
  const trimmed = value.trim()
  if (!trimmed) return { ok: true, value: null }
  if (SHELL_META.test(trimmed)) return { ok: false }
  if (isAbsolutePath(trimmed) || isCommandName(trimmed)) {
    return { ok: true, value: trimmed }
  }
  return { ok: false }
}

function blankToNull(value: string | null): string | null {
  if (value == null) return null
  const trimmed = value.trim()
  return trimmed ? trimmed : null
}

function uniqueLanguages(languages: string[]): string[] {
  const seen = new Set<string>()
  const next: string[] = []
  for (const language of languages) {
    const key = language.trim()
    if (!key || seen.has(key)) continue
    seen.add(key)
    next.push(key)
  }
  return next
}

function toPayload(
  draft: CodeIntelConfig,
  binaryPath: string | null
): CodeIntelConfig {
  const modes =
    draft.serena.modes.length > 0
      ? [...draft.serena.modes]
      : [...DEFAULT_SERENA_MODES]
  return {
    enabled: draft.enabled,
    lsp: {
      enabled: draft.lsp.enabled,
      languages: uniqueLanguages(draft.lsp.languages),
      max_concurrent: clampConcurrent(draft.lsp.max_concurrent),
      auto_install: draft.lsp.auto_install,
    },
    codegraph: {
      enabled: draft.codegraph.enabled,
      auto_install: draft.codegraph.auto_install,
      binary_path: binaryPath,
    },
    serena: {
      enabled: draft.serena.enabled,
      auto_install: draft.serena.auto_install,
      command: blankToNull(draft.serena.command),
      version: draft.serena.version.trim() || SERENA_PIN,
      context: draft.serena.context.trim() || "codex",
      modes,
    },
  }
}

function providerStatusKey(provider: CodeIntelProviderStatus): StatusKey {
  switch (provider.runtime) {
    case "handshake_failed":
      return "handshakeFailed"
    case "exited":
      return "exited"
    case "waiting":
      return "waiting"
    case "starting":
      return "starting"
    case "connected":
      return "connected"
    case "stopped":
      break
  }
  if (provider.install === "failed") return "installFailed"
  if (provider.install === "downloading") return "downloading"
  if (provider.discovery === "missing" && provider.install !== "installed") {
    return "notInstalled"
  }
  if (provider.discovery === "unconfigured" && provider.install === "idle") {
    return "unconfigured"
  }
  if (provider.runtime === "stopped") return "stopped"
  if (provider.discovery === "global") return "global"
  if (provider.discovery === "managed") return "managed"
  if (provider.discovery === "override") return "override"
  if (provider.install === "installed") return "installed"
  return "stopped"
}

function languageProviderKey(provider: string): StatusKey | null {
  switch (provider) {
    case "handshake_failed":
      return "handshakeFailed"
    case "exited":
      return "exited"
    case "waiting":
      return "waiting"
    case "starting":
      return "starting"
    case "connected":
      return "connected"
    case "stopped":
      return "stopped"
    case "failed":
      return "installFailed"
    case "downloading":
      return "downloading"
    case "installed":
      return "installed"
    case "missing":
      return "notInstalled"
    case "unconfigured":
      return "unconfigured"
    case "global":
      return "global"
    case "managed":
      return "managed"
    case "override":
      return "override"
    default:
      return null
  }
}

function statusText(t: Translator, key: StatusKey): string {
  switch (key) {
    case "connected":
      return t("status.connected")
    case "stopped":
      return t("status.stopped")
    case "starting":
      return t("status.starting")
    case "exited":
      return t("status.exited")
    case "handshakeFailed":
      return t("status.handshakeFailed")
    case "waiting":
      return t("status.waiting")
    case "notInstalled":
      return t("status.notInstalled")
    case "unconfigured":
      return t("status.unconfigured")
    case "installFailed":
      return t("status.installFailed")
    case "downloading":
      return t("status.downloading")
    case "installed":
      return t("status.installed")
    case "global":
      return t("status.global")
    case "managed":
      return t("status.managed")
    case "override":
      return t("status.override")
  }
}

function statusBadgeVariant(
  key: StatusKey
): "secondary" | "outline" | "destructive" {
  if (key === "connected" || key === "installed") return "secondary"
  if (
    key === "handshakeFailed" ||
    key === "installFailed" ||
    key === "exited"
  ) {
    return "destructive"
  }
  return "outline"
}

function providerFailed(provider: CodeIntelProviderStatus): boolean {
  return (
    provider.runtime === "handshake_failed" ||
    provider.runtime === "exited" ||
    provider.install === "failed"
  )
}

function fallbackProvider(id: CodeIntelProviderId): CodeIntelProviderStatus {
  return {
    id,
    configured: false,
    discovery: "unconfigured",
    install: "idle",
    runtime: "stopped",
    version: null,
    resolved_command: null,
    last_error: null,
    last_started_at: null,
  }
}

function providerById(
  providers: CodeIntelProviderStatus[],
  id: CodeIntelProviderId
): CodeIntelProviderStatus {
  return (
    providers.find((provider) => provider.id === id) ?? fallbackProvider(id)
  )
}

function toolEnabled(draft: CodeIntelConfig, id: CodeIntelProviderId): boolean {
  if (id === "lsp") return draft.lsp.enabled
  if (id === "codegraph") return draft.codegraph.enabled
  return draft.serena.enabled
}

function withToolEnabled(
  draft: CodeIntelConfig,
  id: CodeIntelProviderId,
  enabled: boolean
): CodeIntelConfig {
  if (id === "lsp") return { ...draft, lsp: { ...draft.lsp, enabled } }
  if (id === "codegraph") {
    return { ...draft, codegraph: { ...draft.codegraph, enabled } }
  }
  return { ...draft, serena: { ...draft.serena, enabled } }
}

function toolTitle(t: Translator, id: CodeIntelProviderId): string {
  if (id === "lsp") return t("lspTitle")
  if (id === "codegraph") return t("codegraphTitle")
  return t("serenaTitle")
}

function toolDescription(t: Translator, id: CodeIntelProviderId): string {
  if (id === "lsp") return t("lspDescription")
  if (id === "codegraph") return t("codegraphDescription")
  return t("serenaDescription")
}

function languageRows(
  reported: CodeIntelLanguageStatus[],
  selected: string[]
): CodeIntelLanguageStatus[] {
  const seen = new Set(reported.map((row) => row.language))
  const extras = selected
    .filter((language) => language.trim() && !seen.has(language))
    .map((language) => ({
      language,
      label: language,
      checked: true,
      detected: false,
      provider: "unconfigured",
    }))
  return [...reported, ...extras]
}

function formatArgv(argv: readonly string[]): string {
  return argv
    .map((arg) =>
      arg.length === 0 || /[\s"]/.test(arg)
        ? `"${arg.replace(/"/g, '\\"')}"`
        : arg
    )
    .join(" ")
}

function serenaArgv(
  serena: CodeIntelConfig["serena"],
  project: string,
  provider: CodeIntelProviderStatus
): string[] {
  const modes =
    serena.modes.length > 0 ? serena.modes : [...DEFAULT_SERENA_MODES]
  const context = serena.context.trim() || "codex"
  const tail = [
    "start-mcp-server",
    "--context",
    context,
    "--project",
    project,
    ...modes.flatMap((mode) => ["--mode", mode]),
    "--enable-web-dashboard",
    "false",
    "--open-web-dashboard",
    "false",
  ]
  const override = serena.command?.trim()
  if (override) return [override, ...tail]
  if (provider.resolved_command) return [provider.resolved_command, ...tail]
  const version = serena.version.trim() || SERENA_PIN
  if (provider.discovery === "missing" && serena.auto_install) {
    return [
      "uvx",
      "--from",
      `git+https://github.com/oraios/serena@${version}`,
      "serena",
      ...tail,
    ]
  }
  return ["serena", ...tail]
}

function codegraphArgv(
  binary: string | null,
  resolved: string | null
): string[] {
  const parsed = normalizeCommandSpec(binary)
  const fromInput = parsed.ok ? parsed.value : null
  const bin = fromInput || resolved?.trim() || "codegraph"
  return [bin, "serve", "--mcp"]
}

function listedContexts(current: string): readonly string[] {
  if ((SERENA_CONTEXTS as readonly string[]).includes(current)) {
    return SERENA_CONTEXTS
  }
  return current ? [current, ...SERENA_CONTEXTS] : SERENA_CONTEXTS
}

export function CodeIntelligenceSettings() {
  const t = useTranslations("CodeIntelligenceSettings")
  const activeFolder = useAppWorkspaceStore((s) => {
    const id = s.activeFolderId
    return id == null ? null : (s.allFolders.find((f) => f.id === id) ?? null)
  })
  const storeCwd = activeFolder?.path ?? null

  const [loading, setLoading] = useState(true)
  const [refreshing, setRefreshing] = useState(false)
  const [saving, setSaving] = useState(false)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [status, setStatus] = useState<CodeIntelStatus | null>(null)
  const [draft, setDraft] = useState<CodeIntelConfig | null>(null)
  const [baseline, setBaseline] = useState<CodeIntelConfig | null>(null)
  const [selected, setSelected] = useState<CodeIntelProviderId>("lsp")

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
      applyStatus(await getCodeIntelStatus(storeCwd))
    } catch (err) {
      setLoadError(toErrorMessage(err))
    } finally {
      setLoading(false)
    }
  }, [applyStatus, storeCwd])

  useEffect(() => {
    void load()
  }, [load])

  const refresh = useCallback(async () => {
    setRefreshing(true)
    try {
      applyStatus(await getCodeIntelStatus(storeCwd))
    } catch (err) {
      setLoadError(toErrorMessage(err))
    } finally {
      setRefreshing(false)
    }
  }, [applyStatus, storeCwd])

  const retry = useCallback(async () => {
    setRefreshing(true)
    try {
      applyStatus(await retryCodeIntel(storeCwd))
    } catch (err) {
      setLoadError(toErrorMessage(err))
    } finally {
      setRefreshing(false)
    }
  }, [applyStatus, storeCwd])

  const updateDraft = useCallback((next: CodeIntelConfig) => {
    setSaveError(null)
    setDraft(next)
  }, [])

  const dirty =
    draft != null &&
    baseline != null &&
    JSON.stringify(draft) !== JSON.stringify(baseline)

  const save = useCallback(async () => {
    if (!draft) return
    const binary = normalizeCommandSpec(draft.codegraph.binary_path)
    if (!binary.ok) {
      setSaveError(t("binaryPathInvalid"))
      return
    }
    const payload = toPayload(draft, binary.value)
    setSaving(true)
    setSaveError(null)
    try {
      await setCodeIntelSettings(payload)
      applyStatus(await getCodeIntelStatus(storeCwd))
    } catch (err) {
      setSaveError(toErrorMessage(err))
    } finally {
      setSaving(false)
    }
  }, [applyStatus, draft, storeCwd, t])

  if (loading && !draft) {
    return (
      <div className="flex h-full items-center justify-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="h-4 w-4 animate-spin" />
        {t("loading")}
      </div>
    )
  }

  const workspace = status?.cwd ?? storeCwd
  const binaryInvalid = draft
    ? !normalizeCommandSpec(draft.codegraph.binary_path).ok
    : false
  const project = workspace ?? t("noWorkspace")

  return (
    <ScrollArea className="h-full">
      <div className="w-full space-y-4 p-3 md:p-4">
        <header className="space-y-1">
          <h1 className="flex items-center gap-2 text-sm font-semibold">
            <Wrench
              className="size-4 text-muted-foreground"
              aria-hidden="true"
            />
            {t("sectionTitle")}
          </h1>
          <p className="text-xs leading-5 text-muted-foreground">
            {t("sectionDescription")}
          </p>
          <p className="text-xs leading-5 text-muted-foreground">
            {t("workspaceLabel")}:{" "}
            <span className="font-mono break-all text-foreground">
              {workspace ?? t("noWorkspace")}
            </span>
          </p>
        </header>

        {loadError ? (
          <SettingsError>
            {t("loadFailed", { message: loadError })}
          </SettingsError>
        ) : null}
        {saveError ? (
          <SettingsError>
            {t("saveFailed", { message: saveError })}
          </SettingsError>
        ) : null}
        {status?.migration_report ? (
          <SettingNote>
            {t("migrationReport")}: {status.migration_report}
          </SettingNote>
        ) : null}

        {draft ? (
          <div
            data-testid="code-tools-columns"
            className="grid grid-cols-1 items-start gap-4 md:grid-cols-2"
          >
            <div className="min-w-0 space-y-4">
              <SettingsSection
                title={t("masterTitle")}
                description={t("masterDescription")}
                htmlFor="code-intel-master"
                control={
                  <Switch
                    id="code-intel-master"
                    checked={draft.enabled}
                    onCheckedChange={(enabled) =>
                      updateDraft({ ...draft, enabled })
                    }
                  />
                }
              />
              <ToolList
                draft={draft}
                providers={status?.providers ?? []}
                selected={selected}
                onSelect={setSelected}
                onToggle={(id, enabled) =>
                  updateDraft(withToolEnabled(draft, id, enabled))
                }
              />
            </div>
            <div className="min-w-0">
              <ToolDetail
                id={selected}
                draft={draft}
                provider={providerById(status?.providers ?? [], selected)}
                rows={languageRows(
                  status?.lsp_languages ?? [],
                  draft.lsp.languages
                )}
                project={project}
                binaryInvalid={binaryInvalid}
                refreshing={refreshing}
                onChange={updateDraft}
                onCheck={() => void refresh()}
                onRetry={() => void retry()}
                onApply={() => void save()}
              />
            </div>
          </div>
        ) : null}

        <SettingsSaveBar
          onSave={() => void save()}
          saving={saving}
          disabled={!dirty}
          label={dirty ? t("saveAndApply") : t("save")}
          savingLabel={t("saving")}
        />
      </div>
    </ScrollArea>
  )
}

function ToolList({
  draft,
  providers,
  selected,
  onSelect,
  onToggle,
}: {
  draft: CodeIntelConfig
  providers: CodeIntelProviderStatus[]
  selected: CodeIntelProviderId
  onSelect: (id: CodeIntelProviderId) => void
  onToggle: (id: CodeIntelProviderId, enabled: boolean) => void
}) {
  const t = useTranslations("CodeIntelligenceSettings")
  return (
    <SettingCard>
      <div className="px-3 py-2 text-xs font-medium text-muted-foreground">
        {t("toolsLabel")}
      </div>
      {TOOL_IDS.map((id) => {
        const provider = providerById(providers, id)
        const key = providerStatusKey(provider)
        const title = toolTitle(t, id)
        const pressed = selected === id
        return (
          <div
            key={id}
            className={cn(
              "flex items-start gap-2 p-3",
              pressed && "bg-muted/80"
            )}
          >
            <button
              type="button"
              className="min-w-0 flex-1 rounded-lg text-start outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
              aria-pressed={pressed}
              onClick={() => onSelect(id)}
            >
              <span className="flex items-center justify-between gap-2">
                <span className="text-sm font-medium">{title}</span>
                <Badge variant={statusBadgeVariant(key)}>
                  {statusText(t, key)}
                </Badge>
              </span>
              <span className="mt-1 block text-xs leading-5 text-muted-foreground">
                {toolDescription(t, id)}
              </span>
            </button>
            <div className="shrink-0 pt-0.5">
              <Switch
                id={`code-intel-tool-${id}`}
                aria-label={title}
                checked={toolEnabled(draft, id)}
                onCheckedChange={(enabled) => onToggle(id, enabled)}
              />
            </div>
          </div>
        )
      })}
    </SettingCard>
  )
}

function ToolDetail({
  id,
  draft,
  provider,
  rows,
  project,
  binaryInvalid,
  refreshing,
  onChange,
  onCheck,
  onRetry,
  onApply,
}: {
  id: CodeIntelProviderId
  draft: CodeIntelConfig
  provider: CodeIntelProviderStatus
  rows: CodeIntelLanguageStatus[]
  project: string
  binaryInvalid: boolean
  refreshing: boolean
  onChange: (next: CodeIntelConfig) => void
  onCheck: () => void
  onRetry: () => void
  onApply: () => void
}) {
  const t = useTranslations("CodeIntelligenceSettings")
  const failed = providerFailed(provider)
  const key = providerStatusKey(provider)
  const version = provider.version

  return (
    <SettingCard>
      <div className="flex items-start justify-between gap-3 p-3">
        <div className="min-w-0">
          <h2 className="text-sm font-medium">{toolTitle(t, id)}</h2>
          <p className="text-xs leading-5 text-muted-foreground">
            {toolDescription(t, id)}
          </p>
        </div>
        <div className="flex shrink-0 flex-col items-end gap-1">
          <span className="text-xs text-muted-foreground">
            {t("statusLabel")}
          </span>
          <Badge variant={statusBadgeVariant(key)}>{statusText(t, key)}</Badge>
        </div>
      </div>

      {version ? (
        <p className="px-3 pb-3 text-xs text-muted-foreground">
          {t("version", { version })}
        </p>
      ) : null}
      {provider.resolved_command ? (
        <p className="px-3 pb-3 text-xs break-all text-muted-foreground">
          {t("resolvedCommand", { command: provider.resolved_command })}
        </p>
      ) : null}

      {id === "lsp" ? (
        <LspFields draft={draft} rows={rows} onChange={onChange} />
      ) : null}
      {id === "codegraph" ? (
        <CodegraphFields
          draft={draft}
          provider={provider}
          binaryInvalid={binaryInvalid}
          onChange={onChange}
        />
      ) : null}
      {id === "serena" ? (
        <SerenaFields
          draft={draft}
          provider={provider}
          project={project}
          onChange={onChange}
        />
      ) : null}

      {failed || provider.last_error ? (
        <details open className="px-3 pb-3 text-xs">
          <summary className="cursor-pointer text-muted-foreground">
            {t("viewLog")}
          </summary>
          <p className="pt-1 break-words text-foreground">
            {provider.last_error ?? t("noLog")}
          </p>
        </details>
      ) : null}

      <LaunchActions
        masterOn={draft.enabled}
        failed={failed}
        refreshing={refreshing}
        onCheck={onCheck}
        onRetry={onRetry}
        onApply={onApply}
      />
    </SettingCard>
  )
}

function LspFields({
  draft,
  rows,
  onChange,
}: {
  draft: CodeIntelConfig
  rows: CodeIntelLanguageStatus[]
  onChange: (next: CodeIntelConfig) => void
}) {
  const t = useTranslations("CodeIntelligenceSettings")

  const toggleLanguage = (language: string, checked: boolean) => {
    const has = draft.lsp.languages.includes(language)
    const languages = checked
      ? has
        ? draft.lsp.languages
        : [...draft.lsp.languages, language]
      : draft.lsp.languages.filter((item) => item !== language)
    onChange({ ...draft, lsp: { ...draft.lsp, languages } })
  }

  return (
    <>
      {rows.length === 0 ? (
        <p className="px-3 py-2 text-xs text-muted-foreground">
          {t("emptyLanguages")}
        </p>
      ) : (
        <>
          <div className="flex items-center gap-3 px-3 pt-3 text-xs text-muted-foreground">
            <span className="min-w-0 flex-1" />
            <span className="shrink-0">{t("detected")}</span>
            <span className="w-28 shrink-0 text-end">{t("providerState")}</span>
          </div>
          <ul>
            {rows.map((row) => {
              const providerKey = languageProviderKey(row.provider)
              const providerLabel = providerKey
                ? statusText(t, providerKey)
                : row.provider
              return (
                <li
                  key={row.language}
                  data-language={row.language}
                  className="flex items-center gap-3 px-3 py-2"
                >
                  <label className="flex min-w-0 flex-1 items-center gap-2 text-sm">
                    <input
                      type="checkbox"
                      className="size-3.5 accent-primary"
                      checked={draft.lsp.languages.includes(row.language)}
                      onChange={(event) =>
                        toggleLanguage(row.language, event.target.checked)
                      }
                    />
                    <span className="truncate">{row.label}</span>
                  </label>
                  <span className="shrink-0 text-xs text-muted-foreground">
                    {row.detected ? t("detected") : t("notDetected")}
                  </span>
                  <span className="w-28 shrink-0 text-end text-xs">
                    {providerLabel}
                  </span>
                </li>
              )
            })}
          </ul>
        </>
      )}
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
              onChange({
                ...draft,
                lsp: {
                  ...draft.lsp,
                  max_concurrent: raw === "" ? Number.NaN : Number(raw),
                },
              })
            }}
            className="h-8 w-24 bg-background text-xs"
          />
        }
      />
      <SettingRow
        title={t("autoInstall")}
        description={t("autoInstallDescription")}
        htmlFor="code-intel-lsp-auto-install"
        control={
          <Switch
            id="code-intel-lsp-auto-install"
            checked={draft.lsp.auto_install}
            onCheckedChange={(autoInstall) =>
              onChange({
                ...draft,
                lsp: { ...draft.lsp, auto_install: autoInstall },
              })
            }
          />
        }
      />
    </>
  )
}

function CodegraphFields({
  draft,
  provider,
  binaryInvalid,
  onChange,
}: {
  draft: CodeIntelConfig
  provider: CodeIntelProviderStatus
  binaryInvalid: boolean
  onChange: (next: CodeIntelConfig) => void
}) {
  const t = useTranslations("CodeIntelligenceSettings")
  return (
    <>
      <SettingRow
        title={t("binaryPath")}
        description={t("binaryPathHint")}
        htmlFor="code-intel-binary-path"
      >
        <Input
          id="code-intel-binary-path"
          value={draft.codegraph.binary_path ?? ""}
          placeholder={t("binaryPathPlaceholder")}
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          aria-invalid={binaryInvalid}
          onChange={(event) =>
            onChange({
              ...draft,
              codegraph: {
                ...draft.codegraph,
                binary_path: event.target.value ? event.target.value : null,
              },
            })
          }
        />
        {binaryInvalid ? (
          <p className="text-xs text-destructive">{t("binaryPathInvalid")}</p>
        ) : null}
      </SettingRow>
      <SettingRow
        title={t("autoInstall")}
        description={t("autoInstallDescription")}
        htmlFor="code-intel-codegraph-auto-install"
        control={
          <Switch
            id="code-intel-codegraph-auto-install"
            checked={draft.codegraph.auto_install}
            onCheckedChange={(autoInstall) =>
              onChange({
                ...draft,
                codegraph: { ...draft.codegraph, auto_install: autoInstall },
              })
            }
          />
        }
      />
      <ArgvRow
        argv={codegraphArgv(
          draft.codegraph.binary_path,
          provider.resolved_command
        )}
      />
    </>
  )
}

function SerenaFields({
  draft,
  provider,
  project,
  onChange,
}: {
  draft: CodeIntelConfig
  provider: CodeIntelProviderStatus
  project: string
  onChange: (next: CodeIntelConfig) => void
}) {
  const t = useTranslations("CodeIntelligenceSettings")
  const modes =
    draft.serena.modes.length > 0
      ? draft.serena.modes
      : [...DEFAULT_SERENA_MODES]
  const contexts = listedContexts(draft.serena.context)
  return (
    <>
      <SettingRow title={t("context")} htmlFor="code-intel-serena-context">
        <select
          id="code-intel-serena-context"
          className="border-input bg-background h-8 w-full max-w-xs rounded-4xl border px-3 text-xs outline-none focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50"
          value={draft.serena.context || "codex"}
          onChange={(event) =>
            onChange({
              ...draft,
              serena: { ...draft.serena, context: event.target.value },
            })
          }
        >
          {contexts.map((context) => (
            <option key={context} value={context}>
              {context}
            </option>
          ))}
        </select>
      </SettingRow>
      <SettingRow title={t("modesSummary", { modes: modes.join(", ") })} />
      <SettingRow
        title={t("autoInstall")}
        description={t("autoInstallDescription")}
        htmlFor="code-intel-serena-auto-install"
        control={
          <Switch
            id="code-intel-serena-auto-install"
            checked={draft.serena.auto_install}
            onCheckedChange={(autoInstall) =>
              onChange({
                ...draft,
                serena: { ...draft.serena, auto_install: autoInstall },
              })
            }
          />
        }
      />
      <ArgvRow argv={serenaArgv(draft.serena, project, provider)} />
      <div className="p-3">
        <BrowserLink
          href={SERENA_DOCS}
          className="text-xs text-primary underline-offset-4 hover:underline"
        >
          {t("officialDocs")}
        </BrowserLink>
      </div>
    </>
  )
}

function ArgvRow({ argv }: { argv: readonly string[] }) {
  const t = useTranslations("CodeIntelligenceSettings")
  return (
    <SettingRow title={t("command")}>
      <pre
        translate="no"
        className="overflow-x-auto rounded-lg bg-background/80 p-2 font-mono text-xs break-all whitespace-pre-wrap"
      >
        {formatArgv(argv)}
      </pre>
    </SettingRow>
  )
}

function LaunchActions({
  masterOn,
  failed,
  refreshing,
  onCheck,
  onRetry,
  onApply,
}: {
  masterOn: boolean
  failed: boolean
  refreshing: boolean
  onCheck: () => void
  onRetry: () => void
  onApply: () => void
}) {
  const t = useTranslations("CodeIntelligenceSettings")
  return (
    <div className="flex flex-col gap-2 p-3">
      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={!masterOn || refreshing}
          onClick={onCheck}
        >
          {t("check")}
        </Button>
        {failed ? (
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={!masterOn || refreshing}
            onClick={onRetry}
          >
            {t("retry")}
          </Button>
        ) : (
          <Button
            type="button"
            size="sm"
            disabled={!masterOn || refreshing}
            onClick={onApply}
          >
            {t("installAndStart")}
          </Button>
        )}
      </div>
      <p className="text-xs leading-5 text-muted-foreground">
        {t("launchHint")}
      </p>
    </div>
  )
}
