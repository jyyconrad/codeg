import { openDirectoryInTerminalApp } from "@/lib/api"
import { isLocalDesktop } from "@/lib/platform"

export type DirectoryTerminalLaunch = "builtin" | "external"

/**
 * Open a directory in the chosen terminal app on the local desktop.
 * System default, a remote window, and the browser keep the built-in terminal.
 */
export async function openDirectoryInTerminal(
  path: string
): Promise<DirectoryTerminalLaunch> {
  if (!isLocalDesktop()) return "builtin"
  const result = await openDirectoryInTerminalApp(path)
  return result.launchedExternal ? "external" : "builtin"
}
