import { existsSync } from "node:fs"
import { homedir } from "node:os"
import { delimiter, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

export const SRC_TAURI = fileURLToPath(new URL("..", import.meta.url))
export const TARGET_DIR = join(SRC_TAURI, "target")

// GUI apps may inherit a PATH that finds Homebrew Rust before rustup.
// Keep Cargo/rustc and all Tauri hook children on the project's toolchain.
export function rustEnv(base = process.env) {
  const cargoBin = resolve(base.CARGO_HOME || join(homedir(), ".cargo"), "bin")
  const suffix = process.platform === "win32" ? ".exe" : ""
  for (const tool of ["cargo", "rustc"]) {
    if (!existsSync(join(cargoBin, tool + suffix))) {
      throw new Error(
        `Missing ${tool} in ${cargoBin}; install Rust with rustup.`
      )
    }
  }
  const env = { ...base }
  const pathKey = Object.keys(env).find((key) => key.toUpperCase() === "PATH")
  const oldPath = pathKey ? env[pathKey] : ""
  for (const key of Object.keys(env)) {
    if (key.toUpperCase() === "PATH") delete env[key]
  }
  env.PATH = [cargoBin, oldPath].filter(Boolean).join(delimiter)
  env.CARGO_TARGET_DIR = TARGET_DIR
  env.CARGO_BUILD_BUILD_DIR = TARGET_DIR
  // Cross-compilation must be explicit in CLI arguments/Tauri's target, so
  // a stale shell default cannot silently create another native target tree.
  delete env.CARGO_BUILD_TARGET
  return env
}
