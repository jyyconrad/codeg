import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs"
import { tmpdir } from "node:os"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import test from "node:test"

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url))
const HOST = "aarch64-apple-darwin"
const BUILD_ARGS = [
  "build",
  "--release",
  "--bin",
  "codeg-mcp",
  "--no-default-features",
]

function fixture(t, { host = HOST, built, failBuild = false } = {}) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), "codeg-sidecar-test-")))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const srcTauri = join(root, "src-tauri")
  const scripts = join(srcTauri, "scripts")
  mkdirSync(scripts, { recursive: true })
  for (const name of ["prepare-sidecars.mjs", "rust-env.mjs"]) {
    copyFileSync(join(SCRIPT_DIR, name), join(scripts, name))
  }
  const cargoHome = join(root, "cargo-home")
  mkdirSync(join(cargoHome, "bin"), { recursive: true })
  for (const name of ["cargo", "rustc"]) {
    const suffix = process.platform === "win32" ? ".exe" : ""
    writeFileSync(join(cargoHome, "bin", name + suffix), "")
  }
  if (built) {
    const binary = join(srcTauri, "target", built)
    mkdirSync(dirname(binary), { recursive: true })
    writeFileSync(binary, "new companion binary")
  }

  // Replace only external compilers. The real CLI still selects and copies
  // files in an isolated project, and a compiler is never launched.
  writeFileSync(
    join(scripts, "run-fixture.mjs"),
    `import childProcess from "node:child_process"
import { appendFileSync } from "node:fs"
import { syncBuiltinESMExports } from "node:module"
childProcess.execFileSync = (command, args, options = {}) => {
  appendFileSync(${JSON.stringify(join(root, "calls.jsonl"))}, JSON.stringify({
    command, args, cwd: options.cwd,
    targetDir: options.env?.CARGO_TARGET_DIR,
  }) + "\\n")
  if (command === "rustc") return ${JSON.stringify(`rustc 1.98.1\nhost: ${host}\n`)}
  if (command !== "cargo") throw new Error("unexpected command: " + command)
  if (${failBuild}) throw new Error("fixture cargo build failed")
  return ""
}
syncBuiltinESMExports()
await import("./prepare-sidecars.mjs")
`
  )

  return {
    srcTauri,
    staged: (name) => join(srcTauri, "binaries", name),
    run(args = [], env = {}) {
      const result = spawnSync(
        process.execPath,
        [join(scripts, "run-fixture.mjs"), ...args],
        {
          cwd: root,
          encoding: "utf8",
          env: {
            ...process.env,
            CARGO_HOME: cargoHome,
            CODEG_SKIP_SIDECAR: "0",
            TAURI_TARGET_TRIPLE: "",
            ...env,
          },
        }
      )
      const callsPath = join(root, "calls.jsonl")
      const calls = existsSync(callsPath)
        ? readFileSync(callsPath, "utf8").trim().split("\n").map(JSON.parse)
        : []
      return { ...result, calls }
    },
  }
}

for (const [name, args, env] of [
  ["implicit host", [], {}],
  ["Tauri host environment", [], { TAURI_TARGET_TRIPLE: HOST }],
  ["explicit host", ["--target", HOST], {}],
]) {
  test(`${name} stages native release without another target tree`, (t) => {
    const project = fixture(t, { built: "release/codeg-mcp" })
    const result = project.run(args, env)
    assert.equal(result.status, 0, result.stderr)
    assert.equal(
      readFileSync(project.staged(`codeg-mcp-${HOST}`), "utf8"),
      "new companion binary"
    )
    const cargo = result.calls.find((call) => call.command === "cargo")
    assert.deepEqual(cargo.args, BUILD_ARGS)
    assert.equal(resolve(cargo.cwd), project.srcTauri)
    assert.equal(cargo.targetDir, join(project.srcTauri, "target"))
  })
}

test("cross compilation overrides the Tauri host and retains target directory", (t) => {
  const target = "x86_64-unknown-linux-gnu"
  const project = fixture(t, { built: `${target}/release/codeg-mcp` })
  const result = project.run([`--target=${target}`], {
    TAURI_TARGET_TRIPLE: HOST,
  })
  assert.equal(result.status, 0, result.stderr)
  assert.equal(
    readFileSync(project.staged(`codeg-mcp-${target}`), "utf8"),
    "new companion binary"
  )
  assert.deepEqual(result.calls.find((call) => call.command === "cargo").args, [
    ...BUILD_ARGS,
    "--target",
    target,
  ])
})

test("Windows native staging keeps the exe suffix without a triple directory", (t) => {
  const host = "x86_64-pc-windows-msvc"
  const project = fixture(t, { host, built: "release/codeg-mcp.exe" })
  const result = project.run([], { TAURI_TARGET_TRIPLE: host })
  assert.equal(result.status, 0, result.stderr)
  assert.equal(
    readFileSync(project.staged(`codeg-mcp-${host}.exe`), "utf8"),
    "new companion binary"
  )
})

test("a Tauri cross target selects its directory and Windows exe suffix", (t) => {
  const target = "x86_64-pc-windows-msvc"
  const project = fixture(t, { built: `${target}/release/codeg-mcp.exe` })
  const result = project.run([], { TAURI_TARGET_TRIPLE: target })
  assert.equal(result.status, 0, result.stderr)
  assert.equal(
    readFileSync(project.staged(`codeg-mcp-${target}.exe`), "utf8"),
    "new companion binary"
  )
  assert.deepEqual(result.calls.find((call) => call.command === "cargo").args, [
    ...BUILD_ARGS,
    "--target",
    target,
  ])
})

test("a failed build does not replace a previously staged companion", (t) => {
  const project = fixture(t, {
    built: "release/codeg-mcp",
    failBuild: true,
  })
  const staged = project.staged(`codeg-mcp-${HOST}`)
  mkdirSync(dirname(staged), { recursive: true })
  writeFileSync(staged, "previous companion binary")
  const result = project.run()
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /fixture cargo build failed/)
  assert.equal(readFileSync(staged, "utf8"), "previous companion binary")
})

test("a missing build artifact does not replace a staged companion", (t) => {
  const project = fixture(t)
  const staged = project.staged(`codeg-mcp-${HOST}`)
  mkdirSync(dirname(staged), { recursive: true })
  writeFileSync(staged, "previous companion binary")
  const result = project.run()
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /does not exist/)
  assert.equal(readFileSync(staged, "utf8"), "previous companion binary")
})

test("skip mode neither invokes compilers nor creates a staged file", (t) => {
  const project = fixture(t)
  const result = project.run([], { CODEG_SKIP_SIDECAR: "1" })
  assert.equal(result.status, 0, result.stderr)
  assert.deepEqual(result.calls, [])
  assert.equal(existsSync(project.staged(`codeg-mcp-${HOST}`)), false)
})
