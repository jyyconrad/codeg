#!/usr/bin/env node
import { execFileSync } from "node:child_process"
import { lstatSync, readdirSync, realpathSync } from "node:fs"
import { join } from "node:path"
import { pathToFileURL } from "node:url"
import { rustEnv, SRC_TAURI, TARGET_DIR } from "./rust-env.mjs"

const GIB = 1024 ** 3

export function cacheLimitGiB(
  value = process.env.CODEG_DEBUG_CACHE_LIMIT_GIB ?? "10"
) {
  const limit = Number(value)
  if (!Number.isFinite(limit) || limit <= 0) {
    throw new Error("Debug cache limit must be a positive number of GiB.")
  }
  return limit
}

function statIfPresent(path) {
  try {
    return lstatSync(path)
  } catch (error) {
    if (error.code === "ENOENT") return null
    throw error
  }
}

export function inspectDebugCache(targetDir = TARGET_DIR) {
  const debugDir = join(targetDir, "debug")
  for (const path of [targetDir, debugDir]) {
    if (statIfPresent(path)?.isSymbolicLink()) {
      throw new Error(`Refusing a symlinked cache directory: ${path}`)
    }
  }
  let bytes = 0
  const seen = new Set()
  const visit = (path) => {
    const stat = statIfPresent(path)
    if (!stat || stat.isSymbolicLink()) return
    if (stat.isDirectory()) {
      for (const name of readdirSync(path)) visit(join(path, name))
    } else if (stat.isFile()) {
      const key = `${stat.dev}:${stat.ino}`
      if (stat.ino && seen.has(key)) return
      seen.add(key)
      bytes += typeof stat.blocks === "number" ? stat.blocks * 512 : stat.size
    }
  }
  visit(debugDir)
  return { debugDir, bytes }
}

export function warnDebugCache() {
  const limit = cacheLimitGiB()
  const { bytes } = inspectDebugCache()
  if (bytes > limit * GIB) {
    console.warn(
      `[rust-cache] debug ${(bytes / GIB).toFixed(2)} GiB exceeds ${limit} GiB. Stop build/watch processes, then run pnpm rust:cache:prune.`
    )
  }
}

function assertNoBuildProcesses() {
  // Cargo's whole-profile clean does NOT acquire the build lock. This is an
  // explicit maintenance command: stop watchers first, then reject active builds.
  const windows = process.platform === "win32"
  const output = execFileSync(
    windows ? "tasklist" : "ps",
    windows ? ["/FO", "CSV", "/NH"] : ["-axo", "comm="],
    { encoding: "utf8" }
  )
  const busy = output.split(/\r?\n/).some((line) => {
    const name = windows
      ? line.split('","')[0].replace(/^"/, "")
      : line.trim().split("/").pop()
    return /^(cargo|rustc|clippy-driver|cargo-tauri)(\.exe)?$/i.test(name)
  })
  if (busy)
    throw new Error(
      "A Rust build is running. Stop build/watch processes before pruning debug."
    )
}

function main() {
  let prune = false
  let dryRun = false
  let limit = cacheLimitGiB()
  const args = process.argv.slice(2)
  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--prune") prune = true
    else if (args[i] === "--dry-run") dryRun = true
    else if (args[i] === "--limit-gib" && args[i + 1])
      limit = cacheLimitGiB(args[++i])
    else
      throw new Error(
        "Usage: rust-cache.mjs [--prune] [--dry-run] [--limit-gib N]"
      )
  }
  const { debugDir, bytes } = inspectDebugCache()
  console.log(
    `[rust-cache] ${debugDir}: ${(bytes / GIB).toFixed(2)} GiB; budget ${limit} GiB`
  )
  if (!prune || bytes <= limit * GIB) return
  if (dryRun) {
    console.log(`[rust-cache] Would clean ${debugDir}; no files deleted.`)
    return
  }
  assertNoBuildProcesses()
  execFileSync(
    "cargo",
    [
      "clean",
      "--manifest-path",
      join(SRC_TAURI, "Cargo.toml"),
      "--profile",
      "dev",
      "--frozen",
    ],
    {
      cwd: SRC_TAURI,
      env: rustEnv(),
      stdio: "inherit",
    }
  )
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(realpathSync(process.argv[1])).href
) {
  try {
    main()
  } catch (error) {
    console.error(`[rust-cache] ${error.message}`)
    process.exitCode = 1
  }
}
