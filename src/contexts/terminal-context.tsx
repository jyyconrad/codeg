"use client"

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react"
import {
  getSystemTerminalSettings,
  terminalAgentState,
  terminalKill,
  terminalSnapshot,
} from "@/lib/api"
import { getTransport } from "@/lib/transport"
import { randomUUID } from "@/lib/utils"
import { useActiveFolder } from "@/contexts/active-folder-context"
import { useShortcutSettings } from "@/hooks/use-shortcut-settings"
import { matchShortcutEvent } from "@/lib/keyboard-shortcuts"
import { newerAgentTerminalEvent } from "@/lib/terminal-agent-state"
import {
  parseStoredAgentTerminalTabs,
  markStoredAgentTerminalLaunched,
  serializeAgentTerminalTabs,
  waitForRestorableAgentTerminal,
} from "@/lib/terminal-agent-tabs"
import type {
  AgentTerminalEvent,
  AgentType,
  TerminalAgentHookSupport,
} from "@/lib/types"

export interface TerminalAgentLaunch {
  agentType: AgentType
  prompt: string
  /** Native session to reopen. Absent on a fresh launch. */
  resumeSessionId?: string
}

export interface TerminalTab {
  id: string
  folderId: number
  title: string
  workingDir: string
  shell?: string
  initialCommand?: string
  agentLaunch?: TerminalAgentLaunch
  agentAttach?: boolean
  agentType?: AgentType
  agentRun?: {
    runId: string
    hookSupport: TerminalAgentHookSupport
    latestEvent?: AgentTerminalEvent
    latestEventType?: AgentTerminalEvent["type"]
  }
}

const DEFAULT_HEIGHT = 300
const MIN_HEIGHT = 150
const MAX_HEIGHT = 600
const TERMINAL_SETTINGS_UPDATED_EVENT = "app://terminal-settings-updated"
const AGENT_TERMINAL_TABS_STORAGE_KEY = "codeg:agent-terminal-tabs"

interface TerminalContextValue {
  isOpen: boolean
  height: number
  minHeight: number
  maxHeight: number
  toggle: () => void
  setHeight: (h: number) => void
  tabs: TerminalTab[]
  activeTabId: string | null
  exitedTerminals: Set<string>
  markTerminalExited: (id: string) => void
  markAgentLaunched: (
    id: string,
    runId: string,
    hookSupport: TerminalAgentHookSupport,
    latestEvent?: AgentTerminalEvent
  ) => void
  markAgentLaunchFailed: (id: string, error: unknown) => void
  isTerminalExplicitlyClosed: (id: string) => boolean
  createTerminal: () => Promise<void>
  createTerminalInDirectory: (
    workingDir: string,
    title?: string,
    shell?: string
  ) => Promise<string | null>
  createTerminalWithCommand: (
    title: string,
    command: string
  ) => Promise<string | null>
  createAgentTerminal: (
    agentType: AgentType,
    workingDir: string,
    prompt: string,
    title: string
  ) => Promise<string | null>
  closeTerminal: (id: string) => void
  closeOtherTerminals: (id: string) => void
  closeAllTerminals: () => void
  renameTerminal: (id: string, title: string) => void
  switchTerminal: (id: string) => void
}

const TerminalContext = createContext<TerminalContextValue | null>(null)

export function useTerminalContext() {
  const ctx = useContext(TerminalContext)
  if (!ctx) {
    throw new Error("useTerminalContext must be used within TerminalProvider")
  }
  return ctx
}

export function TerminalProvider({ children }: { children: ReactNode }) {
  const { activeFolder, activeFolderId } = useActiveFolder()
  const { shortcuts } = useShortcutSettings()
  const [isOpen, setIsOpen] = useState(false)
  const [height, setHeightState] = useState(DEFAULT_HEIGHT)
  const [tabs, setTabs] = useState<TerminalTab[]>([])
  const [activeTabId, setActiveTabId] = useState<string | null>(null)
  const tabCounterRef = useRef(0)
  const [exitedTerminals, setExitedTerminals] = useState<Set<string>>(new Set())
  const [defaultTerminalShell, setDefaultTerminalShell] = useState<
    string | null
  >(null)
  const lastMouseActivityInTerminalRef = useRef(false)
  // Keep a ref of tabs for cleanup on unmount (effect [] captures stale state)
  const tabsRef = useRef(tabs)
  const pendingAgentLaunchesRef = useRef(
    new Map<
      string,
      { resolve: (id: string) => void; reject: (error: unknown) => void }
    >()
  )
  const bufferedAgentEventsRef = useRef(new Map<string, AgentTerminalEvent>())
  const explicitlyClosedTerminalIdsRef = useRef(new Set<string>())
  const [agentTabsRestored, setAgentTabsRestored] = useState(false)
  useEffect(() => {
    tabsRef.current = tabs
  }, [tabs])

  const folderPath = activeFolder?.path ?? ""
  const currentFolderId = activeFolderId ?? 0
  const resolveTerminalShell = useCallback(
    (shell?: string) => shell ?? defaultTerminalShell ?? undefined,
    [defaultTerminalShell]
  )

  useEffect(() => {
    let cancelled = false
    let stored = [] as ReturnType<typeof parseStoredAgentTerminalTabs>
    try {
      stored = parseStoredAgentTerminalTabs(
        typeof window === "undefined"
          ? null
          : window.sessionStorage.getItem(AGENT_TERMINAL_TABS_STORAGE_KEY)
      )
    } catch {
      // Session storage can be unavailable in hardened/private contexts.
    }
    void Promise.all(
      stored.map(async (tab) => {
        const restorable = await waitForRestorableAgentTerminal(
          tab,
          async () => {
            const [snapshot, state] = await Promise.all([
              terminalSnapshot(tab.id).catch(() => null),
              terminalAgentState(tab.id).catch(() => null),
            ])
            return { snapshot, state }
          }
        )
        if (!restorable) return null
        const { state } = restorable
        return {
          ...tab,
          folderId: 0,
          agentAttach: true,
          agentRun: {
            runId: state.run_id,
            hookSupport: state.hook_support,
            latestEvent: state.latest_event ?? undefined,
            latestEventType: state.latest_event?.type,
          },
        } satisfies TerminalTab
      })
    ).then((restored) => {
      if (cancelled) return
      const live: TerminalTab[] = []
      for (const tab of restored) {
        if (tab != null) live.push(tab)
      }
      setTabs((current) => [...live, ...current])
      if (live.length > 0) {
        setIsOpen(true)
        setActiveTabId((current) => current ?? live[0].id)
      }
      setAgentTabsRestored(true)
    })
    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    if (!agentTabsRestored || typeof window === "undefined") return
    const stored = tabs.flatMap((tab) => {
      const agentType = tab.agentLaunch?.agentType ?? tab.agentType
      if (!agentType || (!tab.agentLaunch && !tab.agentAttach)) return []
      return [
        {
          id: tab.id,
          title: tab.title,
          workingDir: tab.workingDir,
          ...(tab.shell ? { shell: tab.shell } : {}),
          agentType,
        },
      ]
    })
    try {
      window.sessionStorage.setItem(
        AGENT_TERMINAL_TABS_STORAGE_KEY,
        serializeAgentTerminalTabs(stored)
      )
    } catch {
      // Best effort only; the live tab remains usable without persistence.
    }
  }, [agentTabsRestored, tabs])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined

    getSystemTerminalSettings()
      .then((settings) => {
        if (!cancelled) setDefaultTerminalShell(settings.default_shell)
      })
      .catch((err) => {
        console.error("[terminal] load terminal settings failed:", err)
      })

    getTransport()
      .subscribe<{ default_shell: string | null }>(
        TERMINAL_SETTINGS_UPDATED_EVENT,
        (settings) => {
          setDefaultTerminalShell(settings.default_shell)
        }
      )
      .then((dispose) => {
        if (cancelled) {
          dispose()
          return
        }
        unlisten = dispose
      })
      .catch((err) => {
        console.error("[terminal] subscribe terminal settings failed:", err)
      })

    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [])

  useEffect(() => {
    let disposed = false
    let unlisten: (() => void) | undefined
    getTransport()
      .subscribe<AgentTerminalEvent>("agent-terminal://event", (event) => {
        const eventKey = `${event.terminal_id}:${event.run_id}`
        const latestEvent = newerAgentTerminalEvent(
          bufferedAgentEventsRef.current.get(eventKey),
          event
        )
        if (latestEvent) {
          bufferedAgentEventsRef.current.set(eventKey, latestEvent)
        }
        setTabs((current) =>
          current.map((tab) =>
            tab.id === event.terminal_id && tab.agentRun?.runId === event.run_id
              ? (() => {
                  const mergedEvent = newerAgentTerminalEvent(
                    tab.agentRun.latestEvent,
                    event
                  )
                  return {
                    ...tab,
                    agentRun: {
                      ...tab.agentRun,
                      latestEvent: mergedEvent,
                      latestEventType: mergedEvent?.type,
                    },
                  }
                })()
              : tab
          )
        )
      })
      .then((dispose) => {
        if (disposed) dispose()
        else unlisten = dispose
      })
      .catch(() => {})
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [])

  const markTerminalExited = useCallback((id: string) => {
    setExitedTerminals((prev) => {
      if (prev.has(id)) return prev
      const next = new Set(prev)
      next.add(id)
      return next
    })
  }, [])

  const markAgentLaunched = useCallback(
    (
      id: string,
      runId: string,
      hookSupport: TerminalAgentHookSupport,
      snapshotEvent?: AgentTerminalEvent
    ) => {
      const eventKey = `${id}:${runId}`
      const latestEvent = newerAgentTerminalEvent(
        snapshotEvent,
        bufferedAgentEventsRef.current.get(eventKey)
      )
      setTabs((current) =>
        current.map((tab) =>
          tab.id === id
            ? markStoredAgentTerminalLaunched(tab, {
                runId,
                hookSupport,
                latestEvent,
              })
            : tab
        )
      )
      pendingAgentLaunchesRef.current.get(id)?.resolve(id)
      pendingAgentLaunchesRef.current.delete(id)
    },
    []
  )

  const markAgentLaunchFailed = useCallback((id: string, error: unknown) => {
    pendingAgentLaunchesRef.current.get(id)?.reject(error)
    pendingAgentLaunchesRef.current.delete(id)
  }, [])

  const isTerminalExplicitlyClosed = useCallback(
    (id: string) => explicitlyClosedTerminalIdsRef.current.has(id),
    []
  )

  const removeExitedTerminals = useCallback((ids: string[]) => {
    setExitedTerminals((prev) => {
      if (prev.size === 0) return prev
      let changed = false
      const next = new Set(prev)
      for (const id of ids) {
        if (next.delete(id)) changed = true
      }
      return changed ? next : prev
    })
  }, [])

  const killTerminalTabs = useCallback((targetTabs: TerminalTab[]) => {
    targetTabs.forEach((tab) => {
      terminalKill(tab.id).catch(() => {})
    })
  }, [])

  const rejectPendingAgentTabs = useCallback(
    (targetTabs: TerminalTab[], reason: string) => {
      targetTabs.forEach((tab) => {
        markAgentLaunchFailed(tab.id, new Error(reason))
      })
    },
    [markAgentLaunchFailed]
  )

  const toggle = useCallback(() => {
    const autoId = randomUUID()
    const nextCounter = tabCounterRef.current + 1

    setIsOpen((wasOpen) => !wasOpen)

    // Auto-create first terminal when opening with no tabs
    setTabs((currentTabs) => {
      if (currentTabs.length > 0 || !folderPath) return currentTabs
      tabCounterRef.current = nextCounter
      return [
        {
          id: autoId,
          folderId: currentFolderId,
          title: `Terminal ${nextCounter}`,
          workingDir: folderPath,
          shell: resolveTerminalShell(),
        },
      ]
    })

    setActiveTabId((prev) => {
      if (prev !== null) return prev
      if (!folderPath) return null
      return autoId
    })
  }, [folderPath, currentFolderId, resolveTerminalShell])

  const createTerminalWithCommand = useCallback(
    async (title: string, command: string) => {
      if (!folderPath) return null

      setIsOpen(true)

      const id = randomUUID()
      tabCounterRef.current += 1
      setTabs((prev) => [
        ...prev,
        {
          id,
          folderId: currentFolderId,
          title,
          workingDir: folderPath,
          shell: resolveTerminalShell(),
          initialCommand: command,
        },
      ])
      setActiveTabId(id)

      return id
    },
    [folderPath, currentFolderId, resolveTerminalShell]
  )

  const createTerminalInDirectory = useCallback(
    async (workingDir: string, title?: string, shell?: string) => {
      if (!workingDir) return null

      setIsOpen(true)

      const id = randomUUID()
      tabCounterRef.current += 1
      const defaultTitle = `Terminal ${tabCounterRef.current}`
      setTabs((prev) => [
        ...prev,
        {
          id,
          folderId: currentFolderId,
          title: title ?? defaultTitle,
          workingDir,
          shell: resolveTerminalShell(shell),
        },
      ])
      setActiveTabId(id)

      return id
    },
    [currentFolderId, resolveTerminalShell]
  )

  const createAgentTerminal = useCallback(
    async (
      agentType: AgentType,
      workingDir: string,
      prompt: string,
      title: string
    ) => {
      if (!workingDir || !prompt.trim()) return null
      setIsOpen(true)
      const id = randomUUID()
      explicitlyClosedTerminalIdsRef.current.delete(id)
      tabCounterRef.current += 1
      setTabs((prev) => [
        ...prev,
        {
          id,
          folderId: currentFolderId,
          title,
          workingDir,
          shell: resolveTerminalShell(),
          agentLaunch: { agentType, prompt },
          agentType,
        },
      ])
      setActiveTabId(id)
      return new Promise<string>((resolve, reject) => {
        pendingAgentLaunchesRef.current.set(id, { resolve, reject })
      })
    },
    [currentFolderId, resolveTerminalShell]
  )

  const createTerminal = useCallback(async () => {
    if (!folderPath) return
    await createTerminalInDirectory(folderPath)
  }, [folderPath, createTerminalInDirectory])

  const setHeight = useCallback((h: number) => {
    setHeightState(Math.max(MIN_HEIGHT, Math.min(MAX_HEIGHT, h)))
  }, [])

  const closeTerminal = useCallback(
    (id: string) => {
      explicitlyClosedTerminalIdsRef.current.add(id)
      markAgentLaunchFailed(
        id,
        new Error("Agent terminal closed before launch")
      )
      markTerminalExited(id)
      removeExitedTerminals([id])
      terminalKill(id).catch(() => {})
      setTabs((prev) => {
        const next = prev.filter((t) => t.id !== id)
        if (next.length === 0) {
          tabCounterRef.current = 0
          setIsOpen(false)
          setActiveTabId(null)
        } else {
          setActiveTabId((prevActive) =>
            prevActive === id ? next[next.length - 1].id : prevActive
          )
        }
        return next
      })
    },
    [markAgentLaunchFailed, markTerminalExited, removeExitedTerminals]
  )

  const closeOtherTerminals = useCallback(
    (id: string) => {
      setTabs((prev) => {
        const closed = prev.filter((t) => t.id !== id)
        closed.forEach((tab) =>
          explicitlyClosedTerminalIdsRef.current.add(tab.id)
        )
        rejectPendingAgentTabs(closed, "Agent terminal closed before launch")
        killTerminalTabs(closed)
        removeExitedTerminals(closed.map((t) => t.id))
        return prev.filter((t) => t.id === id)
      })
      setActiveTabId(id)
    },
    [killTerminalTabs, rejectPendingAgentTabs, removeExitedTerminals]
  )

  const closeAllTerminals = useCallback(() => {
    setTabs((prev) => {
      prev.forEach((tab) => explicitlyClosedTerminalIdsRef.current.add(tab.id))
      rejectPendingAgentTabs(prev, "Agent terminal closed before launch")
      killTerminalTabs(prev)
      removeExitedTerminals(prev.map((t) => t.id))
      return []
    })
    tabCounterRef.current = 0
    setActiveTabId(null)
    setIsOpen(false)
  }, [killTerminalTabs, rejectPendingAgentTabs, removeExitedTerminals])

  const renameTerminal = useCallback((id: string, title: string) => {
    setTabs((prev) => prev.map((t) => (t.id === id ? { ...t, title } : t)))
  }, [])

  const switchTerminal = useCallback((id: string) => {
    setActiveTabId(id)
  }, [])

  const isInTerminalRegion = useCallback((target: EventTarget | null) => {
    if (!(target instanceof Element)) return false
    return Boolean(target.closest('[data-terminal-panel-region="true"]'))
  }, [])

  const updateLastMouseActivity = useCallback(
    (target: EventTarget | null) => {
      const next = isInTerminalRegion(target)
      if (lastMouseActivityInTerminalRef.current === next) return
      lastMouseActivityInTerminalRef.current = next
    },
    [isInTerminalRegion]
  )

  useEffect(() => {
    const handlePointerActivity = (event: PointerEvent) => {
      updateLastMouseActivity(event.target)
    }
    const handleFocusActivity = (event: FocusEvent) => {
      updateLastMouseActivity(event.target)
    }

    window.addEventListener("pointerover", handlePointerActivity, true)
    window.addEventListener("pointerdown", handlePointerActivity, true)
    window.addEventListener("focusin", handleFocusActivity, true)
    return () => {
      window.removeEventListener("pointerover", handlePointerActivity, true)
      window.removeEventListener("pointerdown", handlePointerActivity, true)
      window.removeEventListener("focusin", handleFocusActivity, true)
    }
  }, [updateLastMouseActivity])

  useEffect(() => {
    if (!isOpen) {
      lastMouseActivityInTerminalRef.current = false
    }
  }, [isOpen])

  useEffect(() => {
    const handleTerminalHotkeys = (event: KeyboardEvent) => {
      if (!isOpen) return

      const targetInTerminal = isInTerminalRegion(event.target)
      const activeElementInTerminal = isInTerminalRegion(document.activeElement)
      const shouldHandle =
        lastMouseActivityInTerminalRef.current ||
        targetInTerminal ||
        activeElementInTerminal
      if (!shouldHandle) return

      if (matchShortcutEvent(event, shortcuts.new_terminal_tab)) {
        event.preventDefault()
        event.stopPropagation()
        void createTerminal()
        return
      }

      if (
        activeTabId &&
        matchShortcutEvent(event, shortcuts.close_current_terminal_tab)
      ) {
        event.preventDefault()
        event.stopPropagation()
        closeTerminal(activeTabId)
      }
    }

    window.addEventListener("keydown", handleTerminalHotkeys, true)
    return () => {
      window.removeEventListener("keydown", handleTerminalHotkeys, true)
    }
  }, [
    activeTabId,
    closeTerminal,
    createTerminal,
    isInTerminalRegion,
    isOpen,
    shortcuts.close_current_terminal_tab,
    shortcuts.new_terminal_tab,
  ])

  // Cleanup all terminals on unmount — uses ref to get current tabs
  useEffect(() => {
    const pendingAgentLaunches = pendingAgentLaunchesRef.current
    return () => {
      pendingAgentLaunches.forEach(({ reject }) =>
        reject(new Error("Terminal provider unmounted before agent launch"))
      )
      pendingAgentLaunches.clear()
      tabsRef.current.forEach((t) => {
        if (t.agentLaunch || t.agentAttach) return
        terminalKill(t.id).catch(() => {})
      })
    }
  }, [])

  const value = useMemo(
    () => ({
      isOpen,
      height,
      minHeight: MIN_HEIGHT,
      maxHeight: MAX_HEIGHT,
      toggle,
      setHeight,
      tabs,
      activeTabId,
      exitedTerminals,
      markTerminalExited,
      markAgentLaunched,
      markAgentLaunchFailed,
      isTerminalExplicitlyClosed,
      createTerminal,
      createTerminalInDirectory,
      createTerminalWithCommand,
      createAgentTerminal,
      closeTerminal,
      closeOtherTerminals,
      closeAllTerminals,
      renameTerminal,
      switchTerminal,
    }),
    [
      isOpen,
      height,
      toggle,
      setHeight,
      tabs,
      activeTabId,
      exitedTerminals,
      markTerminalExited,
      markAgentLaunched,
      markAgentLaunchFailed,
      isTerminalExplicitlyClosed,
      createTerminal,
      createTerminalInDirectory,
      createTerminalWithCommand,
      createAgentTerminal,
      closeTerminal,
      closeOtherTerminals,
      closeAllTerminals,
      renameTerminal,
      switchTerminal,
    ]
  )

  return (
    <TerminalContext.Provider value={value}>
      {children}
    </TerminalContext.Provider>
  )
}
