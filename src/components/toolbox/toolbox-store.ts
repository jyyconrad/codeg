import { create } from "zustand"
import {
  loadLastToolboxTool,
  loadToolboxFavorites,
  loadToolboxRecent,
  pushToolboxRecent,
  saveLastToolboxTool,
  saveToolboxFavorites,
} from "./storage"
import type { ToolboxToolId } from "./types"

interface ToolboxStoreState {
  selectedToolId: ToolboxToolId | null
  favorites: ToolboxToolId[]
  recent: ToolboxToolId[]
  /** Text handed from "send result to" — consumed once by the destination tool. */
  pendingInput: string | null
  hydrated: boolean
  hydrate: () => void
  selectTool: (id: ToolboxToolId | null) => void
  toggleFavorite: (id: ToolboxToolId) => void
  consumePendingInput: () => string | null
  sendResultTo: (id: ToolboxToolId, text: string) => void
}

export const useToolboxStore = create<ToolboxStoreState>((set, get) => ({
  selectedToolId: null,
  favorites: [],
  recent: [],
  pendingInput: null,
  hydrated: false,
  hydrate: () => {
    if (get().hydrated) return
    const last = loadLastToolboxTool()
    set({
      hydrated: true,
      favorites: loadToolboxFavorites(),
      recent: loadToolboxRecent(),
      selectedToolId: last,
    })
  },
  selectTool: (id) => {
    saveLastToolboxTool(id)
    if (id) {
      const recent = pushToolboxRecent(id)
      set({ selectedToolId: id, recent })
      return
    }
    set({ selectedToolId: null })
  },
  toggleFavorite: (id) => {
    const current = get().favorites
    const next = current.includes(id)
      ? current.filter((item) => item !== id)
      : [...current, id]
    saveToolboxFavorites(next)
    set({ favorites: next })
  },
  consumePendingInput: () => {
    const value = get().pendingInput
    if (value != null) set({ pendingInput: null })
    return value
  },
  sendResultTo: (id, text) => {
    saveLastToolboxTool(id)
    const recent = pushToolboxRecent(id)
    set({ selectedToolId: id, pendingInput: text, recent })
  },
}))
