#!/usr/bin/env node
import { spawn } from "node:child_process"
import { createRequire } from "node:module"
import { resolve } from "node:path"
import { rustEnv, SRC_TAURI } from "./rust-env.mjs"
import { warnDebugCache } from "./rust-cache.mjs"

const [tool, ...args] = process.argv.slice(2)
try {
  if (tool !== "cargo" && tool !== "tauri") {
    throw new Error("Usage: rust-run.mjs <cargo|tauri> [arguments...]")
  }
  if (["build", "check", "test", "clippy", "run", "dev"].includes(args[0])) {
    warnDebugCache()
  }
  const require = createRequire(import.meta.url)
  const command = tool === "cargo" ? "cargo" : process.execPath
  const commandArgs =
    tool === "cargo"
      ? args
      : [require.resolve("@tauri-apps/cli/tauri.js"), ...args]
  const child = spawn(command, commandArgs, {
    cwd: tool === "cargo" ? SRC_TAURI : resolve(SRC_TAURI, ".."),
    env: rustEnv(),
    stdio: "inherit",
  })
  for (const signal of ["SIGINT", "SIGTERM"]) {
    process.on(signal, () => child.kill(signal))
  }
  child.on("error", (error) => {
    console.error(`[rust-run] ${error.message}`)
    process.exitCode = 1
  })
  child.on("exit", (code, signal) => {
    process.exitCode = code ?? (signal === "SIGINT" ? 130 : 1)
  })
} catch (error) {
  console.error(`[rust-run] ${error.message}`)
  process.exitCode = 1
}
