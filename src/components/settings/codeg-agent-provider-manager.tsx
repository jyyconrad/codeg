"use client"

import { useCallback, useEffect, useMemo, useState } from "react"
import {
  Eye,
  EyeOff,
  Loader2,
  Minus,
  Plus,
  Search,
  Star,
  Trash2,
} from "lucide-react"
import { useTranslations } from "next-intl"
import { toast } from "sonner"

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { ScrollArea } from "@/components/ui/scroll-area"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import {
  acpFetchKimiModels,
  createModelProvider,
  deleteModelProvider,
  listModelProviders,
  updateModelProvider,
} from "@/lib/api"
import { toErrorMessage } from "@/lib/app-error"
import {
  catalogFromProviderModel,
  CODEG_CATALOG_DEFAULT_WINDOW,
  CODEG_REQUEST_PROTOCOLS,
  parseCodegRequestProtocol,
  serializeCodegAgentCatalog,
  type CodegAgentCatalog,
  type CodegCatalogModel,
} from "@/lib/codeg-agent-catalog"
import {
  CODEG_PROVIDER_PRESETS,
  codegProviderPreset,
  isLoopbackHttpUrl,
  type CodegProviderPresetId,
} from "@/lib/codeg-agent-providers"
import { getAgentLabel } from "@/lib/custom-agents"
import { completionsModelIdFromProvider } from "@/lib/types"
import type { ModelProviderInfo } from "@/lib/types"
import { cn } from "@/lib/utils"

function providerLetter(name: string): string {
  const letter = name.trim().charAt(0)
  return letter ? letter.toUpperCase() : "?"
}

function ModelWindowInput({
  value,
  label,
  onCommit,
  id,
}: {
  value: number
  label: string
  onCommit: (next: number) => void
  id?: string
}) {
  const [text, setText] = useState(String(value))
  useEffect(() => {
    setText(String(value))
  }, [value])
  return (
    <Input
      id={id}
      type="number"
      min={1}
      className="h-8 w-28"
      aria-label={label}
      value={text}
      onChange={(event) => setText(event.target.value)}
      onBlur={() => {
        const next = Number.parseInt(text, 10)
        if (!Number.isFinite(next) || next <= 0) {
          setText(String(value))
          return
        }
        if (next !== value) onCommit(next)
      }}
    />
  )
}

function modelsOf(provider: ModelProviderInfo): CodegCatalogModel[] {
  const catalog = catalogFromProviderModel(provider.model)
  if (catalog) return catalog.models
  const id = completionsModelIdFromProvider(provider)
  if (!id) return []
  return [
    {
      id,
      name: id,
      context_window: CODEG_CATALOG_DEFAULT_WINDOW,
    },
  ]
}

function catalogOf(provider: ModelProviderInfo): CodegAgentCatalog {
  return (
    catalogFromProviderModel(provider.model) ?? {
      kind: "codeg_agent_catalog",
      version: 1,
      protocol: "chat_completions",
      default: modelsOf(provider)[0]?.id ?? "",
      models: modelsOf(provider),
    }
  )
}

export function CodegAgentProviderManager({
  providers,
  boundProviderId,
  selectedProviderId,
  onSelectProvider,
  onBindProvider,
  onProvidersChanged,
  className,
  bindSwitchId,
  windowInputId,
}: {
  providers: ModelProviderInfo[]
  boundProviderId: number | null
  selectedProviderId: number | null
  onSelectProvider: (id: number) => void
  onBindProvider: (provider: ModelProviderInfo | null) => void
  onProvidersChanged: (
    providers: ModelProviderInfo[],
    touched?: ModelProviderInfo
  ) => void
  className?: string
  bindSwitchId?: string
  windowInputId?: string
}) {
  const t = useTranslations("CodegAgentSettings")
  const tPresets = useTranslations("ModelProviderSettings")
  const [query, setQuery] = useState("")
  const [addOpen, setAddOpen] = useState(false)
  const [deleteTarget, setDeleteTarget] = useState<ModelProviderInfo | null>(
    null
  )
  const [saving, setSaving] = useState(false)
  const [detecting, setDetecting] = useState(false)
  const [draftName, setDraftName] = useState("")
  const [draftUrl, setDraftUrl] = useState("")
  const [draftKey, setDraftKey] = useState("")
  const [showKey, setShowKey] = useState(false)
  const [newModelId, setNewModelId] = useState("")
  const [syncedId, setSyncedId] = useState<number | null>(null)

  const selected =
    providers.find((row) => row.id === selectedProviderId) ?? null
  const canEditCatalog = selected?.agent_type === "codeg_agent"
  const bound = selected != null && selected.id === boundProviderId

  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase()
    if (!needle) return providers
    return providers.filter((row) => {
      const hay = `${row.name} ${row.api_url} ${row.model ?? ""}`.toLowerCase()
      return hay.includes(needle)
    })
  }, [providers, query])

  const selectedModels = selected ? modelsOf(selected) : []

  const refresh = useCallback(
    async (touched?: ModelProviderInfo) => {
      const rows = await listModelProviders()
      onProvidersChanged(rows, touched)
    },
    [onProvidersChanged]
  )

  const persistCatalog = useCallback(
    async (provider: ModelProviderInfo, catalog: CodegAgentCatalog) => {
      if (provider.agent_type !== "codeg_agent") return
      if (catalog.models.length === 0) {
        toast.error(t("emptyModels"))
        return
      }
      setSaving(true)
      try {
        const { provider: updated, affectedRunningSessions } =
          await updateModelProvider({
            id: provider.id,
            model: serializeCodegAgentCatalog(catalog),
          })
        if (affectedRunningSessions > 0) {
          toast.info(
            tPresets("affectedRunningSessions", {
              count: affectedRunningSessions,
            })
          )
        }
        await refresh(updated)
      } catch (err) {
        toast.error(toErrorMessage(err))
      } finally {
        setSaving(false)
      }
    },
    [refresh, t, tPresets]
  )

  const persistCredentials = useCallback(async () => {
    if (!selected || !canEditCatalog) return
    if (!draftName.trim()) {
      toast.error(tPresets("nameRequired"))
      return
    }
    if (!draftUrl.trim()) {
      toast.error(tPresets("apiUrlRequired"))
      return
    }
    if (!draftKey.trim() && !selected.api_key && !isLoopbackHttpUrl(draftUrl)) {
      toast.error(tPresets("apiKeyRequired"))
      return
    }
    setSaving(true)
    try {
      const { provider: updated, affectedRunningSessions } =
        await updateModelProvider({
          id: selected.id,
          name: draftName.trim(),
          apiUrl: draftUrl.trim(),
          apiKey: draftKey.trim() || undefined,
        })
      toast.success(t("providerSaved"))
      if (affectedRunningSessions > 0) {
        toast.info(
          tPresets("affectedRunningSessions", {
            count: affectedRunningSessions,
          })
        )
      }
      setDraftKey("")
      await refresh(updated)
    } catch (err) {
      toast.error(toErrorMessage(err))
    } finally {
      setSaving(false)
    }
  }, [
    canEditCatalog,
    draftKey,
    draftName,
    draftUrl,
    refresh,
    selected,
    t,
    tPresets,
  ])

  const handleDetect = useCallback(async () => {
    if (!selected) return
    const url = (canEditCatalog ? draftUrl : selected.api_url).trim()
    const key =
      draftKey.trim() ||
      selected.api_key.trim() ||
      (isLoopbackHttpUrl(url) ? "local" : "")
    if (!url) {
      toast.error(tPresets("apiUrlRequired"))
      return
    }
    if (!key) {
      toast.error(tPresets("apiKeyRequired"))
      return
    }
    setDetecting(true)
    try {
      const ids = await acpFetchKimiModels({ baseUrl: url, apiKey: key })
      toast.success(t("detectSuccess", { count: ids.length }))
      if (!canEditCatalog || ids.length === 0) return
      const catalog = catalogOf(selected)
      const have = new Set(catalog.models.map((row) => row.id))
      const added: CodegCatalogModel[] = ids
        .filter((id) => id.trim() && !have.has(id.trim()))
        .map((id) => ({
          id: id.trim(),
          name: id.trim(),
          context_window: CODEG_CATALOG_DEFAULT_WINDOW,
        }))
      if (added.length === 0) return
      const next: CodegAgentCatalog = {
        ...catalog,
        models: [...catalog.models, ...added],
        default: catalog.default || added[0].id,
      }
      await persistCatalog(selected, next)
    } catch (err) {
      toast.error(t("detectFailed", { message: toErrorMessage(err) }))
    } finally {
      setDetecting(false)
    }
  }, [
    canEditCatalog,
    draftKey,
    draftUrl,
    persistCatalog,
    selected,
    t,
    tPresets,
  ])

  const addModel = useCallback(async () => {
    if (!selected || !canEditCatalog) return
    const id = newModelId.trim()
    if (!id) return
    const catalog = catalogOf(selected)
    if (catalog.models.some((row) => row.id === id)) {
      setNewModelId("")
      return
    }
    const next: CodegAgentCatalog = {
      ...catalog,
      models: [
        ...catalog.models,
        {
          id,
          name: id,
          context_window: CODEG_CATALOG_DEFAULT_WINDOW,
        },
      ],
      default: catalog.default || id,
    }
    setNewModelId("")
    await persistCatalog(selected, next)
  }, [canEditCatalog, newModelId, persistCatalog, selected])

  useEffect(() => {
    const provider = providers.find((row) => row.id === selectedProviderId)
    if (!provider || provider.id === syncedId) return
    setDraftName(provider.name)
    setDraftUrl(provider.api_url)
    setDraftKey("")
    setShowKey(false)
    setNewModelId("")
    setSyncedId(provider.id)
  }, [providers, selectedProviderId, syncedId])

  const selectRow = useCallback(
    (provider: ModelProviderInfo) => {
      onSelectProvider(provider.id)
    },
    [onSelectProvider]
  )

  return (
    <div
      className={cn(
        "grid min-h-[32rem] overflow-hidden rounded-xl border border-border/70 bg-card lg:grid-cols-[17.5rem_1fr]",
        className
      )}
    >
      <aside className="flex min-h-0 flex-col border-b border-border/70 lg:border-r lg:border-b-0">
        <div className="border-b border-border/70 p-3">
          <div className="relative">
            <Search className="pointer-events-none absolute top-1/2 left-3 size-3.5 -translate-y-1/2 text-muted-foreground" />
            <Input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("searchProviders")}
              className="pl-8"
              aria-label={t("searchProviders")}
            />
          </div>
        </div>
        <ScrollArea className="min-h-0 flex-1">
          <div className="p-2">
            {visible.length === 0 ? (
              <p className="px-2 py-6 text-center text-xs text-muted-foreground">
                {t("noProviders")}
              </p>
            ) : (
              <ul className="space-y-1">
                {visible.map((provider) => {
                  const active = provider.id === selected?.id
                  const on = provider.id === boundProviderId
                  return (
                    <li key={provider.id}>
                      <button
                        type="button"
                        onClick={() => selectRow(provider)}
                        className={cn(
                          "flex w-full items-center gap-2 rounded-xl px-2 py-2 text-left transition-colors",
                          active ? "bg-muted" : "hover:bg-muted/60"
                        )}
                      >
                        <span className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-background text-xs font-semibold ring-1 ring-border">
                          {providerLetter(provider.name)}
                        </span>
                        <span className="min-w-0 flex-1">
                          <span className="block truncate text-sm font-medium">
                            {provider.name}
                          </span>
                          {provider.agent_type !== "codeg_agent" ? (
                            <span className="block truncate text-[11px] text-muted-foreground">
                              {getAgentLabel(provider.agent_type)}
                            </span>
                          ) : null}
                        </span>
                        {on ? (
                          <span className="rounded-full border border-emerald-500/50 px-1.5 py-0.5 text-[10px] font-semibold tracking-wide text-emerald-600 dark:text-emerald-400">
                            {t("providerOn")}
                          </span>
                        ) : null}
                      </button>
                    </li>
                  )
                })}
              </ul>
            )}
          </div>
        </ScrollArea>
        <div className="border-t border-border/70 p-3">
          <Button
            type="button"
            size="sm"
            className="w-full"
            onClick={() => setAddOpen(true)}
          >
            <Plus className="size-3.5" />
            {t("addProvider")}
          </Button>
        </div>
      </aside>

      <section className="min-h-0">
        {selected ? (
          <ScrollArea className="h-full max-h-[40rem]">
            <div className="space-y-5 p-4 md:p-5">
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <h2 className="truncate text-base font-semibold">
                    {selected.name}
                  </h2>
                  <p className="mt-0.5 text-xs text-muted-foreground">
                    {canEditCatalog
                      ? t("presetHint")
                      : t("foreignProviderHint")}
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-xs text-muted-foreground">
                    {t("bindThisProvider")}
                  </span>
                  <Switch
                    id={bindSwitchId}
                    checked={bound}
                    onCheckedChange={(checked) =>
                      onBindProvider(checked ? selected : null)
                    }
                    aria-label={t("bindThisProvider")}
                  />
                </div>
              </div>

              <div className="space-y-3">
                {canEditCatalog ? (
                  <div className="space-y-1.5">
                    <Label htmlFor="codeg-provider-name">
                      {t("providerName")}
                    </Label>
                    <Input
                      id="codeg-provider-name"
                      value={draftName}
                      onChange={(event) => setDraftName(event.target.value)}
                    />
                  </div>
                ) : null}

                <div className="space-y-1.5">
                  <div className="flex items-center justify-between gap-2">
                    <Label htmlFor="codeg-provider-key">{t("apiKey")}</Label>
                    {isLoopbackHttpUrl(
                      canEditCatalog ? draftUrl : selected.api_url
                    ) ? (
                      <span className="text-[11px] text-muted-foreground">
                        {t("loopbackKeyOptional")}
                      </span>
                    ) : null}
                  </div>
                  <div className="flex gap-2">
                    <div className="relative min-w-0 flex-1">
                      <Input
                        id="codeg-provider-key"
                        type={showKey ? "text" : "password"}
                        value={draftKey}
                        onChange={(event) => setDraftKey(event.target.value)}
                        placeholder={
                          selected.api_key_masked || t("apiKeyPlaceholder")
                        }
                        disabled={!canEditCatalog}
                        className="pr-10"
                        autoComplete="off"
                      />
                      <button
                        type="button"
                        className="absolute top-1/2 right-2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                        onClick={() => setShowKey((value) => !value)}
                        aria-label={t("apiKey")}
                      >
                        {showKey ? (
                          <EyeOff className="size-3.5" />
                        ) : (
                          <Eye className="size-3.5" />
                        )}
                      </button>
                    </div>
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      className="shrink-0"
                      disabled={detecting}
                      onClick={() => {
                        void handleDetect()
                      }}
                    >
                      {detecting ? (
                        <Loader2 className="size-3.5 animate-spin" />
                      ) : null}
                      {detecting ? t("detecting") : t("detect")}
                    </Button>
                  </div>
                </div>

                <div className="space-y-1.5">
                  <Label htmlFor="codeg-provider-url">{t("apiUrl")}</Label>
                  <Input
                    id="codeg-provider-url"
                    value={canEditCatalog ? draftUrl : selected.api_url}
                    onChange={(event) => setDraftUrl(event.target.value)}
                    disabled={!canEditCatalog}
                  />
                  <p className="text-[11px] leading-4 text-muted-foreground">
                    {t("apiUrlHint")}
                  </p>
                </div>

                {canEditCatalog ? (
                  <div className="space-y-1.5">
                    <Label htmlFor="codeg-provider-protocol">
                      {t("requestProtocol")}
                    </Label>
                    <Select
                      value={catalogOf(selected).protocol}
                      onValueChange={(value) => {
                        const catalog = catalogOf(selected)
                        void persistCatalog(selected, {
                          ...catalog,
                          protocol: parseCodegRequestProtocol(value),
                        })
                      }}
                    >
                      <SelectTrigger id="codeg-provider-protocol">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {CODEG_REQUEST_PROTOCOLS.map((protocol) => (
                          <SelectItem key={protocol} value={protocol}>
                            {protocol === "chat_completions"
                              ? t("protocolChatCompletions")
                              : protocol === "responses"
                                ? t("protocolResponses")
                                : t("protocolAuto")}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <p className="text-[11px] leading-4 text-muted-foreground">
                      {t("requestProtocolHint")}
                    </p>
                  </div>
                ) : null}

                {canEditCatalog ? (
                  <div className="flex justify-end">
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      disabled={saving}
                      onClick={() => {
                        void persistCredentials()
                      }}
                    >
                      {saving ? (
                        <Loader2 className="size-3.5 animate-spin" />
                      ) : null}
                      {t("saveProvider")}
                    </Button>
                  </div>
                ) : null}
              </div>

              <div className="space-y-2 border-t border-border/70 pt-4">
                <div className="flex items-center justify-between gap-2">
                  <h3 className="text-sm font-medium">
                    {t("modelsTitle")}
                    <span className="ml-1.5 text-xs font-normal text-muted-foreground">
                      {selectedModels.length}
                    </span>
                  </h3>
                </div>
                {selectedModels.length === 0 ? (
                  <p className="rounded-xl border border-dashed border-border/70 px-3 py-6 text-center text-xs text-muted-foreground">
                    {t("emptyModels")}
                  </p>
                ) : (
                  <ul className="space-y-2">
                    {selectedModels.map((model) => {
                      const isDefault = catalogOf(selected).default === model.id
                      return (
                        <li
                          key={model.id}
                          className="flex items-center gap-2 rounded-xl border border-border/70 bg-muted/40 px-3 py-2"
                        >
                          <span className="min-w-0 flex-1 truncate text-sm">
                            {model.id}
                          </span>
                          {canEditCatalog ? (
                            <ModelWindowInput
                              id={isDefault ? windowInputId : undefined}
                              value={model.context_window}
                              label={t("modelWindow")}
                              onCommit={(nextWindow) => {
                                const catalog = catalogOf(selected)
                                void persistCatalog(selected, {
                                  ...catalog,
                                  models: catalog.models.map((row) =>
                                    row.id === model.id
                                      ? { ...row, context_window: nextWindow }
                                      : row
                                  ),
                                })
                              }}
                            />
                          ) : null}
                          {canEditCatalog ? (
                            <Button
                              type="button"
                              size="icon-xs"
                              variant={isDefault ? "default" : "ghost"}
                              aria-label={t("setDefault")}
                              title={t("setDefault")}
                              onClick={() => {
                                const catalog = catalogOf(selected)
                                void persistCatalog(selected, {
                                  ...catalog,
                                  default: model.id,
                                })
                              }}
                            >
                              <Star className="size-3.5" />
                            </Button>
                          ) : isDefault ? (
                            <span className="text-[11px] text-muted-foreground">
                              {t("defaultModel")}
                            </span>
                          ) : null}
                          {canEditCatalog ? (
                            <Button
                              type="button"
                              size="icon-xs"
                              variant="ghost"
                              aria-label={t("removeModel")}
                              disabled={selectedModels.length <= 1}
                              onClick={() => {
                                const catalog = catalogOf(selected)
                                const models = catalog.models.filter(
                                  (row) => row.id !== model.id
                                )
                                void persistCatalog(selected, {
                                  ...catalog,
                                  models,
                                  default:
                                    catalog.default === model.id
                                      ? (models[0]?.id ?? "")
                                      : catalog.default,
                                })
                              }}
                            >
                              <Minus className="size-3.5" />
                            </Button>
                          ) : null}
                        </li>
                      )
                    })}
                  </ul>
                )}
                {canEditCatalog ? (
                  <form
                    className="flex gap-2"
                    onSubmit={(event) => {
                      event.preventDefault()
                      void addModel()
                    }}
                  >
                    <Input
                      value={newModelId}
                      onChange={(event) => setNewModelId(event.target.value)}
                      placeholder={t("modelIdPlaceholder")}
                      aria-label={t("modelIdPlaceholder")}
                    />
                    <Button type="submit" size="sm" variant="outline">
                      <Plus className="size-3.5" />
                      {t("addModel")}
                    </Button>
                  </form>
                ) : null}
              </div>

              {canEditCatalog ? (
                <div className="flex justify-end border-t border-border/70 pt-3">
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    className="text-destructive hover:text-destructive"
                    onClick={() => setDeleteTarget(selected)}
                  >
                    <Trash2 className="size-3.5" />
                    {t("deleteProvider")}
                  </Button>
                </div>
              ) : null}
            </div>
          </ScrollArea>
        ) : (
          <div className="flex h-full min-h-[16rem] items-center justify-center p-6 text-sm text-muted-foreground">
            {t("selectProvider")}
          </div>
        )}
      </section>

      <AddCodegProviderDialog
        open={addOpen}
        onOpenChange={setAddOpen}
        onCreated={async (created) => {
          const rows = await listModelProviders()
          onProvidersChanged(rows, created)
          onSelectProvider(created.id)
          if (boundProviderId == null) onBindProvider(created)
        }}
      />

      <AlertDialog
        open={!!deleteTarget}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("deleteConfirmTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("deleteConfirmDescription")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{tPresets("cancel")}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (!deleteTarget) return
                void (async () => {
                  try {
                    await deleteModelProvider(deleteTarget.id)
                    toast.success(t("providerDeleted"))
                    if (deleteTarget.id === boundProviderId) {
                      onBindProvider(null)
                    }
                    setDeleteTarget(null)
                    await refresh()
                  } catch (err) {
                    const message = toErrorMessage(err)
                    const prefix = "PROVIDER_IN_USE:"
                    if (message.includes(prefix)) {
                      toast.error(
                        tPresets("deleteBlockedByAgent", {
                          agents: message.substring(
                            message.indexOf(prefix) + prefix.length
                          ),
                        })
                      )
                    } else {
                      toast.error(message)
                    }
                  }
                })()
              }}
            >
              {t("deleteProvider")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

function AddCodegProviderDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onCreated: (provider: ModelProviderInfo) => void | Promise<void>
}) {
  const t = useTranslations("CodegAgentSettings")
  const tPresets = useTranslations("ModelProviderSettings")
  const [name, setName] = useState("")
  const [apiUrl, setApiUrl] = useState("")
  const [apiKey, setApiKey] = useState("")
  const [model, setModel] = useState("")
  const [presetId, setPresetId] = useState<CodegProviderPresetId>("custom")
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const reset = useCallback(() => {
    setName("")
    setApiUrl("")
    setApiKey("")
    setModel("")
    setPresetId("custom")
    setError(null)
  }, [])

  const applyPreset = useCallback(
    (id: CodegProviderPresetId) => {
      const preset = codegProviderPreset(id)
      if (!preset) return
      setPresetId(id)
      setApiUrl(preset.apiUrl)
      const presetNames = new Set(
        CODEG_PROVIDER_PRESETS.map((row) =>
          tPresets(`presets.${row.id}` as Parameters<typeof tPresets>[0])
        )
      )
      if (id !== "custom" && (!name.trim() || presetNames.has(name.trim()))) {
        setName(tPresets(`presets.${id}` as Parameters<typeof tPresets>[0]))
      }
    },
    [name, tPresets]
  )

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) reset()
        onOpenChange(next)
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("addProvider")}</DialogTitle>
        </DialogHeader>
        <div className="space-y-3">
          <div className="space-y-1.5">
            <Label>{t("preset")}</Label>
            <Select
              value={presetId}
              onValueChange={(value) =>
                applyPreset(value as CodegProviderPresetId)
              }
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {CODEG_PROVIDER_PRESETS.map((row) => (
                  <SelectItem key={row.id} value={row.id}>
                    {tPresets(
                      `presets.${row.id}` as Parameters<typeof tPresets>[0]
                    )}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="codeg-add-name">{t("providerName")}</Label>
            <Input
              id="codeg-add-name"
              value={name}
              onChange={(event) => setName(event.target.value)}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="codeg-add-url">{t("apiUrl")}</Label>
            <Input
              id="codeg-add-url"
              value={apiUrl}
              onChange={(event) => setApiUrl(event.target.value)}
              placeholder="https://api.openai.com/v1"
            />
            <p className="text-[11px] text-muted-foreground">
              {t("apiUrlHint")}
            </p>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="codeg-add-key">{t("apiKey")}</Label>
            <Input
              id="codeg-add-key"
              type="password"
              value={apiKey}
              onChange={(event) => setApiKey(event.target.value)}
              autoComplete="off"
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="codeg-add-model">{t("addFirstModel")}</Label>
            <Input
              id="codeg-add-model"
              value={model}
              onChange={(event) => setModel(event.target.value)}
              placeholder={t("modelIdPlaceholder")}
            />
          </div>
          {error ? <p className="text-xs text-destructive">{error}</p> : null}
        </div>
        <DialogFooter>
          <Button
            type="button"
            disabled={saving}
            onClick={() => {
              if (!name.trim()) {
                setError(tPresets("nameRequired"))
                return
              }
              if (!apiUrl.trim()) {
                setError(tPresets("apiUrlRequired"))
                return
              }
              if (!apiKey.trim() && !isLoopbackHttpUrl(apiUrl)) {
                setError(tPresets("apiKeyRequired"))
                return
              }
              setSaving(true)
              setError(null)
              const first = model.trim()
              const payload = first
                ? serializeCodegAgentCatalog({
                    kind: "codeg_agent_catalog",
                    version: 1,
                    protocol: "chat_completions",
                    default: first,
                    models: [
                      {
                        id: first,
                        name: first,
                        context_window: CODEG_CATALOG_DEFAULT_WINDOW,
                      },
                    ],
                  })
                : null
              void createModelProvider({
                name: name.trim(),
                apiUrl: apiUrl.trim(),
                apiKey: apiKey.trim(),
                agentType: "codeg_agent",
                model: payload,
              })
                .then(async (created) => {
                  toast.success(t("providerCreated"))
                  reset()
                  onOpenChange(false)
                  await onCreated(created)
                })
                .catch((err: unknown) => {
                  setError(toErrorMessage(err))
                })
                .finally(() => setSaving(false))
            }}
          >
            {saving ? <Loader2 className="size-3.5 animate-spin" /> : null}
            {saving ? t("creating") : t("addProvider")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
