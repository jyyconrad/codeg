/**
 * 能力视图的语义入口，复用 WikiNotesView 展示已归纳的能力笔记。
 * 由 WikiPage 负责导航；能力生成、证据归属及持久化由后端流水线承担。
 */
"use client"
import { WikiNotesView } from "./wiki-notes-view"
export function WikiCapabilitiesView() {
  return <WikiNotesView view="capabilities" />
}
