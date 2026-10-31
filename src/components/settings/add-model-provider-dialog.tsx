"use client"

import { useCallback, useMemo, useState } from "react"
import { Loader2 } from "lucide-react"
import { useTranslations } from "next-intl"
import { toast } from "sonner"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { createModelProvider } from "@/lib/api"
import {
  CODEG_PROVIDER_PRESETS,
  codegProviderPreset,
  isLoopbackHttpUrl,
  type CodegProviderPresetId,
} from "@/lib/codeg-agent-providers"
import { CodexModelListEditor } from "@/components/settings/codex-model-list-editor"
import type { CodexCatalogEndpoint } from "@/components/settings/codex-catalog-source"
import {
  describeCodexSaveError,
  type CodexCatalogTranslator,
} from "@/lib/codex-catalog-error"
import {
  MODEL_PROVIDER_AGENT_TYPES,
  serializeClaudeProviderModel,
  serializeCodexModelConfig,
  type AgentType,
  type ClaudeProviderModel,
  type CodexModelConfig,
  type ModelProviderInfo,
} from "@/lib/types"
import { getAgentLabel } from "@/lib/custom-agents"
import { ClaudeProviderModelFields } from "@/components/settings/claude-model-fields"
import { ModelIdInputWithFetch } from "@/components/settings/provider-models"
import { useProviderModels } from "@/hooks/use-provider-models"

interface AddModelProviderDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  onProviderAdded: (provider: ModelProviderInfo) => void
  defaultAgentType?: AgentType
}

export function AddModelProviderDialog({
  open,
  onOpenChange,
  onProviderAdded,
  defaultAgentType,
}: AddModelProviderDialogProps) {
  const t = useTranslations("ModelProviderSettings")
  const tCatalog = useTranslations(
    "CodexModelEditor.catalogSource"
  ) as unknown as CodexCatalogTranslator
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const initialAgentType = defaultAgentType ?? MODEL_PROVIDER_AGENT_TYPES[0]
  const [name, setName] = useState("")
  const [apiUrl, setApiUrl] = useState("")
  const [apiKey, setApiKey] = useState("")
  const [agentType, setAgentType] = useState<AgentType>(initialAgentType)
  const [presetId, setPresetId] = useState<CodegProviderPresetId>("custom")
  const [singleModel, setSingleModel] = useState("")
  const [claudeModel, setClaudeModel] = useState<ClaudeProviderModel>({})
  const [codexModel, setCodexModel] = useState<CodexModelConfig>({
    customs: [],
  })

  const resetForm = useCallback(() => {
    setName("")
    setApiUrl("")
    setApiKey("")
    setAgentType(defaultAgentType ?? MODEL_PROVIDER_AGENT_TYPES[0])
    setPresetId("custom")
    setSingleModel("")
    setClaudeModel({})
    setCodexModel({ customs: [] })
    setError(null)
  }, [defaultAgentType])

  const handleOpenChange = useCallback(
    (nextOpen: boolean) => {
      if (!nextOpen) resetForm()
      onOpenChange(nextOpen)
    },
    [onOpenChange, resetForm]
  )

  const handleAgentTypeChange = useCallback((next: AgentType) => {
    setAgentType(next)
    setPresetId("custom")
    setSingleModel("")
    setClaudeModel({})
    setCodexModel({ customs: [] })
  }, [])

  const applyPreset = useCallback(
    (id: CodegProviderPresetId) => {
      const preset = codegProviderPreset(id)
      if (!preset) return
      setPresetId(id)
      setApiUrl(preset.apiUrl)
      setSingleModel(preset.model)
      const presetNames = new Set(
        CODEG_PROVIDER_PRESETS.map((row) =>
          t(`presets.${row.id}` as Parameters<typeof t>[0])
        )
      )
      if (id !== "custom" && (!name.trim() || presetNames.has(name.trim()))) {
        setName(t(`presets.${id}` as Parameters<typeof t>[0]))
      }
    },
    [name, t]
  )

  // "Fetch models" asks the provider with the key typed here; nothing is
  // published until a Codex agent is bound to the saved provider.
  const codexEndpoint = useMemo<CodexCatalogEndpoint>(
    () => ({
      baseUrl: apiUrl,
      keySource: apiKey.trim()
        ? { kind: "inline", apiKey: apiKey.trim() }
        : null,
    }),
    [apiUrl, apiKey]
  )

  // Claude and Gemini fields offer the models the provider lists, asked the
  // way each agent sends this key.
  const providerModels = useProviderModels(
    agentType === "claude_code" || agentType === "gemini"
      ? {
          protocol: agentType === "gemini" ? "gemini" : "anthropic",
          baseUrl: apiUrl,
          apiKey,
          keyHeader: agentType === "claude_code" ? "bearer" : null,
        }
      : null
  )

  const modelPlaceholder = useMemo(() => {
    if (agentType === "codex") return t("modelPlaceholderCodex")
    if (agentType === "gemini") return t("modelPlaceholderGemini")
    if (agentType === "codeg_agent") return t("modelPlaceholderCodeg")
    return ""
  }, [agentType, t])

  const localKeyOptional = isLoopbackHttpUrl(apiUrl)

  const handleSubmit = useCallback(async () => {
    if (!name.trim()) {
      setError(t("nameRequired"))
      return
    }
    if (!apiUrl.trim()) {
      setError(t("apiUrlRequired"))
      return
    }
    if (!apiKey.trim() && !isLoopbackHttpUrl(apiUrl)) {
      setError(t("apiKeyRequired"))
      return
    }
    if (!agentType) {
      setError(t("agentTypeRequired"))
      return
    }

    let modelPayload: string | null = null
    if (agentType === "claude_code") {
      modelPayload = serializeClaudeProviderModel(claudeModel)
    } else if (agentType === "codex") {
      modelPayload = serializeCodexModelConfig(codexModel)
    } else if (singleModel.trim()) {
      modelPayload = singleModel.trim()
    }

    setLoading(true)
    setError(null)
    try {
      const created = await createModelProvider({
        name: name.trim(),
        apiUrl: apiUrl.trim(),
        apiKey: apiKey.trim(),
        agentType,
        model: modelPayload,
      })
      toast.success(t("createSuccess"))
      handleOpenChange(false)
      onProviderAdded(created)
    } catch (err: unknown) {
      // A Codex agent bound to the provider gets the change through the
      // model list commit; its errors carry a code to localize.
      setError(describeCodexSaveError(err, tCatalog))
    } finally {
      setLoading(false)
    }
  }, [
    name,
    apiUrl,
    apiKey,
    agentType,
    singleModel,
    claudeModel,
    codexModel,
    handleOpenChange,
    onProviderAdded,
    t,
    tCatalog,
  ])

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("addProvider")}</DialogTitle>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-1.5">
            <label htmlFor="add-mp-name" className="text-xs font-medium">
              {t("providerName")}
            </label>
            <Input
              id="add-mp-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t("providerNamePlaceholder")}
            />
          </div>

          <div className="space-y-1.5">
            <label htmlFor="add-mp-url" className="text-xs font-medium">
              {t("apiUrl")}
            </label>
            <Input
              id="add-mp-url"
              value={apiUrl}
              onChange={(e) => setApiUrl(e.target.value)}
              placeholder={t("apiUrlPlaceholder")}
            />
          </div>

          <div className="space-y-1.5">
            <label htmlFor="add-mp-key" className="text-xs font-medium">
              {t("apiKey")}
            </label>
            <Input
              id="add-mp-key"
              type="password"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder={
                localKeyOptional
                  ? t("apiKeyOptionalLocal")
                  : t("apiKeyPlaceholder")
              }
            />
            {localKeyOptional && (
              <p className="text-2xs text-muted-foreground">
                {t("apiKeyOptionalLocalHint")}
              </p>
            )}
          </div>

          <div className="space-y-1.5">
            <label className="text-xs font-medium">{t("agentType")}</label>
            <Select
              value={agentType}
              onValueChange={(v) => handleAgentTypeChange(v as AgentType)}
            >
              <SelectTrigger className="h-8 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {MODEL_PROVIDER_AGENT_TYPES.map((at) => (
                  <SelectItem key={at} value={at}>
                    {getAgentLabel(at)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          {agentType === "codeg_agent" && (
            <div className="space-y-1.5">
              <label className="text-xs font-medium">
                {t("presets.label")}
              </label>
              <Select
                value={presetId}
                onValueChange={(value) =>
                  applyPreset(value as CodegProviderPresetId)
                }
              >
                <SelectTrigger className="h-8 text-xs">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {CODEG_PROVIDER_PRESETS.map((preset) => (
                    <SelectItem key={preset.id} value={preset.id}>
                      {t(`presets.${preset.id}` as Parameters<typeof t>[0])}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <p className="text-2xs text-muted-foreground">
                {t("presets.hint")}
              </p>
            </div>
          )}

          {agentType === "claude_code" ? (
            <ClaudeProviderModelFields
              value={claudeModel}
              onChange={setClaudeModel}
              state={providerModels}
            />
          ) : agentType === "codex" ? (
            <div className="space-y-1.5">
              <CodexModelListEditor
                value={codexModel}
                onChange={setCodexModel}
                endpoint={codexEndpoint}
              />
            </div>
          ) : (
            <div className="space-y-1.5">
              <label className="text-xs font-medium">{t("model")}</label>
              <ModelIdInputWithFetch
                aria-label={t("model")}
                value={singleModel}
                onChange={setSingleModel}
                state={providerModels}
                placeholder={modelPlaceholder}
              />
            </div>
          )}

          {error && (
            <div className="rounded-md border border-red-500/30 bg-red-500/5 px-3 py-2 text-xs text-red-400">
              {error}
            </div>
          )}
        </div>

        <DialogFooter>
          <Button
            variant="outline"
            onClick={() => handleOpenChange(false)}
            disabled={loading}
          >
            {t("cancel")}
          </Button>
          <Button onClick={handleSubmit} disabled={loading}>
            {loading && <Loader2 className="h-3.5 w-3.5 animate-spin mr-1" />}
            {t("create")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
