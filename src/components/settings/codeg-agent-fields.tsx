"use client"

import { useTranslations } from "next-intl"

import { SettingCard, SettingRow } from "@/components/shared/setting-card"
import { Button } from "@/components/ui/button"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Textarea } from "@/components/ui/textarea"
import { catalogFromProviderModel } from "@/lib/codeg-agent-catalog"
import {
  codegCompactModel,
  patchCodegCompactModel,
} from "@/lib/codeg-agent-env"
import {
  CODEG_BUILTIN_COMPACT_PROMPT,
  CODEG_BUILTIN_SYSTEM_PROMPT,
} from "@/lib/codeg-agent-prompts"
import { completionsModelIdFromProvider } from "@/lib/types"
import type { ModelProviderInfo } from "@/lib/types"

export const CODEG_COMPACT_FOLLOW_SESSION = "__session__"

export function compactModelChoices(provider: ModelProviderInfo | null): {
  id: string
  name: string
}[] {
  if (!provider) return []
  const catalog = catalogFromProviderModel(provider.model)
  if (catalog) {
    return catalog.models.map((model) => ({
      id: model.id,
      name: model.name || model.id,
    }))
  }
  const id = completionsModelIdFromProvider(provider)
  return id ? [{ id, name: id }] : []
}

export function CodegAgentPromptEditors({
  systemPrompt,
  compactPrompt,
  onSystemPromptChange,
  onCompactPromptChange,
  density = "comfortable",
}: {
  systemPrompt: string
  compactPrompt: string
  onSystemPromptChange: (value: string) => void
  onCompactPromptChange: (value: string) => void
  density?: "comfortable" | "compact"
}) {
  const t = useTranslations("CodegAgentSettings")
  const tAgent = useTranslations("AcpAgentSettings.codegAgent")
  const areaClass =
    density === "compact"
      ? "min-h-24 font-mono text-xs leading-5"
      : "min-h-36 font-mono text-xs leading-5"

  const systemEditor = (
    <>
      <Textarea
        value={systemPrompt}
        onChange={(event) => onSystemPromptChange(event.target.value)}
        className={areaClass}
      />
      <Button
        type="button"
        size="xs"
        variant="ghost"
        className="mt-2"
        onClick={() => onSystemPromptChange(CODEG_BUILTIN_SYSTEM_PROMPT)}
      >
        {t("restoreDefault")}
      </Button>
    </>
  )
  const compactEditor = (
    <>
      <Textarea
        value={compactPrompt}
        onChange={(event) => onCompactPromptChange(event.target.value)}
        className={areaClass}
      />
      <Button
        type="button"
        size="xs"
        variant="ghost"
        className="mt-2"
        onClick={() => onCompactPromptChange(CODEG_BUILTIN_COMPACT_PROMPT)}
      >
        {t("restoreDefault")}
      </Button>
    </>
  )

  if (density === "compact") {
    return (
      <div className="space-y-3">
        <div className="space-y-1.5">
          <label className="text-2xs text-muted-foreground">
            {tAgent("systemPrompt")}
          </label>
          {systemEditor}
          <p className="text-2xs text-muted-foreground">
            {tAgent("systemPromptHint")}
          </p>
        </div>
        <div className="space-y-1.5">
          <label className="text-2xs text-muted-foreground">
            {tAgent("compactPrompt")}
          </label>
          {compactEditor}
          <p className="text-2xs text-muted-foreground">
            {tAgent("compactPromptHint")}
          </p>
        </div>
      </div>
    )
  }

  return (
    <SettingCard>
      <SettingRow
        title={tAgent("systemPrompt")}
        description={tAgent("systemPromptHint")}
      >
        {systemEditor}
      </SettingRow>
      <SettingRow
        title={tAgent("compactPrompt")}
        description={tAgent("compactPromptHint")}
      >
        {compactEditor}
      </SettingRow>
    </SettingCard>
  )
}

export function CodegAgentCompactModelField({
  envText,
  onEnvTextChange,
  boundProvider,
  density = "comfortable",
}: {
  envText: string
  onEnvTextChange: (envText: string) => void
  boundProvider: ModelProviderInfo | null
  density?: "comfortable" | "compact"
}) {
  const t = useTranslations("CodegAgentSettings")
  const choices = compactModelChoices(boundProvider)
  const value = codegCompactModel(envText) || CODEG_COMPACT_FOLLOW_SESSION
  const select = (
    <Select
      value={value}
      onValueChange={(next) =>
        onEnvTextChange(
          patchCodegCompactModel(
            envText,
            next === CODEG_COMPACT_FOLLOW_SESSION ? "" : next
          )
        )
      }
      disabled={!boundProvider || choices.length === 0}
    >
      <SelectTrigger className="w-full">
        <SelectValue placeholder={t("compactModelFollowSession")} />
      </SelectTrigger>
      <SelectContent align="start">
        <SelectItem value={CODEG_COMPACT_FOLLOW_SESSION}>
          {t("compactModelFollowSession")}
        </SelectItem>
        {choices.map((model) => (
          <SelectItem key={model.id} value={model.id}>
            {model.name}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )

  if (density === "compact") {
    return (
      <div className="space-y-1.5">
        <label className="text-2xs text-muted-foreground">
          {t("compactModel")}
        </label>
        {select}
        <p className="text-2xs text-muted-foreground">
          {t("compactModelHint")}
        </p>
      </div>
    )
  }

  return (
    <SettingRow title={t("compactModel")} description={t("compactModelHint")}>
      {select}
    </SettingRow>
  )
}
