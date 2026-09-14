/**
 * 工作记录视图的语义入口，复用 WikiNotesView 的工作筛选与阅读流程。
 * 由 WikiPage 切换到此视图，项目归属和笔记事实仍由后端读取模型提供。
 */
"use client"
import { WikiNotesView } from "./wiki-notes-view"
export function WikiWorkView() {
  return <WikiNotesView view="work" />
}
