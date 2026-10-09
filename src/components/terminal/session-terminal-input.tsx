"use client"

import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useState,
} from "react"
import { useTranslations } from "next-intl"
import { TerminalView } from "@/components/terminal/terminal-view"
import { Button } from "@/components/ui/button"
import {
  openSettingsWindow,
  terminalIdForConversation,
  terminalKill,
} from "@/lib/api"
import { claimSessionTerminal } from "@/lib/session-terminal-session"
import { settleTerminalProcessExit } from "@/lib/terminal-session-establish"
import { useTabStore } from "@/stores/tab-store"
import type { AgentType } from "@/lib/types"

interface SessionTerminalInputProps {
  /** Tab id or canvas card key. Closing that session releases the PTY. */
  ownerKey: string
  agentType: AgentType
  agentName: string
  /** Native session to reopen. Stable for the life of this mount. */
  resumeSessionId?: string | null
  /** Row a channel may already be driving in a background PTY. */
  conversationId?: number | null
  workingDir?: string | null
  isActive: boolean
  /**
   * The pane is on screen. A split pane that is visible but not focused still
   * passes true; a keep-alive tab that the group has hidden passes false so
   * the xterm cannot paint through `visibility: hidden`.
   */
  isVisible?: boolean
  /**
   * CLI missing or otherwise unable to launch. Shown only when there is no
   * CLI to draw the screen.
   */
  notice?: string | null
  /** Missing-CLI notices can open Agents settings. Other notices stay put. */
  showSettings?: boolean
  onOpenDiagnostics?: () => void
  diagnosticsLabel?: string
}

/**
 * Replaces the chat composer when an agent starts in the terminal.
 * Mounting it runs that agent's CLI. The CLI owns the screen.
 */
export function SessionTerminalInput({
  ownerKey,
  agentType,
  agentName,
  resumeSessionId,
  conversationId,
  workingDir,
  isActive,
  isVisible = true,
  notice,
  showSettings = true,
  onOpenDiagnostics,
  diagnosticsLabel,
}: SessionTerminalInputProps) {
  const t = useTranslations("Folder.chat.sessionTerminal")
  const agentLaunch = useMemo(
    () => ({
      agentType,
      prompt: "",
      ...(resumeSessionId ? { resumeSessionId } : {}),
    }),
    [agentType, resumeSessionId]
  )
  const directory = workingDir?.trim() ? workingDir : null
  const needsLiveTerminal = conversationId != null || Boolean(resumeSessionId)
  const lookupKey = needsLiveTerminal
    ? `${conversationId ?? ""}|${resumeSessionId ?? ""}`
    : null
  const [lookup, setLookup] = useState<{
    key: string
    id: string | null
  } | null>(null)
  useEffect(() => {
    if (!lookupKey) return
    let cancelled = false
    terminalIdForConversation({
      conversationId: conversationId ?? null,
      nativeSessionId: resumeSessionId ?? null,
    })
      .then((id) => {
        if (!cancelled) setLookup({ key: lookupKey, id })
      })
      .catch(() => {
        if (!cancelled) setLookup({ key: lookupKey, id: null })
      })
    return () => {
      cancelled = true
    }
  }, [lookupKey, conversationId, resumeSessionId])
  const adoptedTerminalId = !lookupKey
    ? null
    : lookup?.key === lookupKey
      ? lookup.id
      : undefined
  const claimed =
    notice || !directory || adoptedTerminalId === undefined
      ? null
      : claimSessionTerminal(ownerKey, agentType, directory, adoptedTerminalId)
  if (claimed?.replacedId) {
    // Agent or directory changed. A repeat render claims the new id and
    // has no replacedId, so the previous process is retired once. Settle
    // the row it already bound first: its exit hook no longer has an owner.
    const replacedId = claimed.replacedId
    const replacedConversationId = claimed.replacedConversationId
    const retire =
      replacedConversationId != null
        ? settleTerminalProcessExit(
            agentType,
            replacedConversationId,
            replacedId
          ).finally(() => terminalKill(replacedId))
        : terminalKill(replacedId)
    void retire.catch(() => {})
  }
  const terminalId = claimed?.terminalId ?? null

  // Mark before paint, not during render: the conversation panel subscribes
  // to this tab. The flag is what stops "new conversation" from reusing the
  // draft before any hook has created a row.
  useLayoutEffect(() => {
    if (!terminalId) return
    useTabStore.getState().markTerminalSession(ownerKey)
  }, [ownerKey, terminalId])

  const handleProcessExited = useCallback(() => {
    useTabStore.getState().clearTerminalSession(ownerKey)
  }, [ownerKey])

  const openSettings = useCallback(() => {
    openSettingsWindow("agents", { agentType }).catch((err) => {
      console.error("[SessionTerminalInput] open settings:", err)
    })
  }, [agentType])

  if (notice) {
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center px-6 py-10 text-center">
        <div role="status" className="max-w-md">
          <p className="text-sm text-balance text-muted-foreground">{notice}</p>
          {showSettings || (onOpenDiagnostics && diagnosticsLabel) ? (
            <div className="mt-4 flex flex-wrap items-center justify-center gap-2">
              {onOpenDiagnostics && diagnosticsLabel ? (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={onOpenDiagnostics}
                >
                  {diagnosticsLabel}
                </Button>
              ) : null}
              {showSettings ? (
                <Button type="button" size="sm" onClick={openSettings}>
                  {t("openSettings")}
                </Button>
              ) : null}
            </div>
          ) : null}
        </div>
      </div>
    )
  }

  return (
    <section
      aria-label={t("region", { agent: agentName })}
      data-terminal-panel-region="true"
      data-session-terminal="true"
      className="relative min-h-0 flex-1"
    >
      {terminalId && directory ? (
        <TerminalView
          key={terminalId}
          terminalId={terminalId}
          workingDir={directory}
          agentLaunch={agentLaunch}
          attach
          isActive={isActive}
          isVisible={isVisible}
          keepVisible
          onProcessExited={handleProcessExited}
        />
      ) : (
        <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
          {t("preparing")}
        </div>
      )}
    </section>
  )
}
