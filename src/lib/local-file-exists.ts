import { pathExists } from "@/lib/api"
import { expandHomePath, isHomeRelativePath } from "@/lib/file-open-target"
import { toAbsoluteFilePath } from "@/lib/file-path-display"

export type LocalFileExistsResult =
  | { ok: true; path: string }
  | { ok: false; reason: "no-workspace" | "not-found" }

/**
 * Resolve a chat file target (absolute, `~/`, or workspace-relative) and
 * confirm it is a real file on the workspace host. Callers toast based on
 * `reason` rather than opening a missing path.
 */
export async function ensureLocalFileExists(
  path: string,
  folderPath: string | null
): Promise<LocalFileExistsResult> {
  let candidate = path.replace(/^\.\/+/, "")
  if (isHomeRelativePath(candidate)) {
    candidate = await expandHomePath(candidate)
  }
  const absolute = toAbsoluteFilePath(candidate, folderPath ?? undefined)
  if (!absolute) return { ok: false, reason: "no-workspace" }
  const exists = await pathExists(absolute)
  if (!exists) return { ok: false, reason: "not-found" }
  return { ok: true, path: absolute }
}
