import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import {
  copyFileSync,
  existsSync,
  linkSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { test } from "node:test"
import { inspectDebugCache } from "./rust-cache.mjs"

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), "codeg-cache-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const src = join(root, "src-tauri")
  mkdirSync(join(src, "scripts"), { recursive: true })
  mkdirSync(join(src, "src"))
  writeFileSync(
    join(src, "Cargo.toml"),
    '[package]\nname="cache-fixture"\nversion="0.1.0"\nedition="2021"\n'
  )
  writeFileSync(join(src, "src", "lib.rs"), "")
  const target = join(src, "target")
  mkdirSync(join(target, "debug"), { recursive: true })
  mkdirSync(join(target, "release", "bundle"), { recursive: true })
  writeFileSync(join(target, "debug", "artifact"), Buffer.alloc(8192, 1))
  writeFileSync(join(target, "release", "bundle", "keep.dmg"), "release")
  return { root, src, target }
}

test(
  "cache budget counts hardlinks once and excludes release and external symlinks",
  {
    skip: process.platform === "win32",
  },
  (t) => {
    const { root, target } = fixture(t)
    linkSync(
      join(target, "debug", "artifact"),
      join(target, "debug", "hardlink")
    )
    writeFileSync(join(root, "outside"), Buffer.alloc(16384, 1))
    symlinkSync(join(root, "outside"), join(target, "debug", "external"))
    const result = inspectDebugCache(target)
    assert.equal(
      result.bytes,
      lstatSync(join(target, "debug", "artifact")).blocks * 512
    )
  }
)

test(
  "missing debug cache is empty and a symlinked target is rejected",
  {
    skip: process.platform === "win32",
  },
  (t) => {
    const { root, target } = fixture(t)
    assert.equal(inspectDebugCache(join(root, "missing")).bytes, 0)
    symlinkSync(target, join(root, "linked"))
    assert.throws(() => inspectDebugCache(join(root, "linked")), /symlink/i)
  }
)

test("prune obeys its budget, supports preview, and preserves release artifacts", (t) => {
  const { src, target } = fixture(t)
  for (const name of ["rust-env.mjs", "rust-cache.mjs"]) {
    copyFileSync(new URL(name, import.meta.url), join(src, "scripts", name))
  }
  // Isolate the process inventory from unrelated builds running on the test
  // machine. Cargo clean itself remains real and operates only on this fixture.
  const processList = join(src, "scripts", "process-list.mjs")
  writeFileSync(
    processList,
    `import cp from "node:child_process"\nimport {syncBuiltinESMExports} from "node:module"\nconst original=cp.execFileSync\ncp.execFileSync=(command,...args)=>command==="ps"||command==="tasklist"?"":original(command,...args)\nsyncBuiltinESMExports()\n`
  )
  const run = (...args) =>
    spawnSync(
      process.execPath,
      [
        "--import",
        processList,
        join(src, "scripts", "rust-cache.mjs"),
        ...args,
      ],
      {
        encoding: "utf8",
        env: process.env,
      }
    )
  const under = run("--prune", "--limit-gib", "1")
  assert.equal(under.status, 0, under.stderr)
  assert.ok(existsSync(join(target, "debug", "artifact")))
  const preview = run("--prune", "--dry-run", "--limit-gib", "0.000001")
  assert.equal(preview.status, 0, preview.stderr)
  assert.ok(existsSync(join(target, "debug", "artifact")))
  const invalid = run("--prune", "--limit-gib", "0")
  assert.notEqual(invalid.status, 0)
  assert.ok(existsSync(join(target, "debug", "artifact")))
  writeFileSync(
    processList,
    readFileSync(processList, "utf8").replace(
      '?"":original',
      '?"cargo":original'
    )
  )
  const busy = run("--prune", "--limit-gib", "0.000001")
  assert.notEqual(busy.status, 0)
  assert.match(busy.stderr, /build is running/)
  assert.ok(existsSync(join(target, "debug", "artifact")))
  writeFileSync(
    processList,
    readFileSync(processList, "utf8").replace(
      '?"cargo":original',
      '?"":original'
    )
  )
  const cleaned = run("--prune", "--limit-gib", "0.000001")
  assert.equal(cleaned.status, 0, cleaned.stderr)
  assert.equal(existsSync(join(target, "debug", "artifact")), false)
  assert.equal(
    readFileSync(join(target, "release", "bundle", "keep.dmg"), "utf8"),
    "release"
  )
})
