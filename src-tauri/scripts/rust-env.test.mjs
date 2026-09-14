import assert from "node:assert/strict"
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { delimiter, join } from "node:path"
import { spawnSync } from "node:child_process"
import { test } from "node:test"
import { fileURLToPath } from "node:url"
import { rustEnv } from "./rust-env.mjs"

test("build children prefer Cargo home and keep output in this repository", (t) => {
  const cargoHome = mkdtempSync(join(tmpdir(), "codeg-cargo-"))
  t.after(() => rmSync(cargoHome, { recursive: true, force: true }))
  mkdirSync(join(cargoHome, "bin"))
  for (const name of ["cargo", "rustc"]) {
    writeFileSync(
      join(
        cargoHome,
        "bin",
        name + (process.platform === "win32" ? ".exe" : "")
      ),
      ""
    )
  }
  const base = {
    CARGO_HOME: cargoHome,
    Path: "/other/bin",
    CARGO_TARGET_DIR: "/unrelated/target",
    CARGO_BUILD_BUILD_DIR: "/unrelated/build",
    CARGO_BUILD_TARGET: "aarch64-apple-darwin",
    RUSTFLAGS: "--cfg example",
  }
  const env = rustEnv(base)
  assert.equal(env.PATH.split(delimiter)[0], join(cargoHome, "bin"))
  assert.equal(env.Path, undefined)
  assert.equal(env.CARGO_BUILD_TARGET, undefined)
  assert.equal(env.RUSTFLAGS, "--cfg example")
  assert.equal(
    env.CARGO_TARGET_DIR,
    fileURLToPath(new URL("../target", import.meta.url))
  )
  assert.equal(env.CARGO_BUILD_BUILD_DIR, env.CARGO_TARGET_DIR)
  assert.equal(base.CARGO_TARGET_DIR, "/unrelated/target")
})

test("missing Rust tools fail instead of falling back to an unrelated compiler", (t) => {
  const cargoHome = mkdtempSync(join(tmpdir(), "codeg-no-rust-"))
  t.after(() => rmSync(cargoHome, { recursive: true, force: true }))
  assert.throws(() => rustEnv({ CARGO_HOME: cargoHome }), /rustup/i)
})

test(
  "Cargo launcher forwards arguments, environment and a failing exit code",
  {
    skip: process.platform === "win32",
  },
  (t) => {
    const cargoHome = mkdtempSync(join(tmpdir(), "codeg-launcher-"))
    t.after(() => rmSync(cargoHome, { recursive: true, force: true }))
    mkdirSync(join(cargoHome, "bin"))
    writeFileSync(join(cargoHome, "bin", "rustc"), "", { mode: 0o755 })
    writeFileSync(
      join(cargoHome, "bin", "cargo"),
      `#!${process.execPath}\nconsole.log(JSON.stringify({args:process.argv.slice(2),cwd:process.cwd(),target:process.env.CARGO_TARGET_DIR}));process.exit(7)\n`,
      { mode: 0o755 }
    )
    const result = spawnSync(
      process.execPath,
      [
        new URL("./rust-run.mjs", import.meta.url).pathname,
        "cargo",
        "check",
        "--no-default-features",
        "--bin",
        "codeg-mcp",
      ],
      { env: { ...process.env, CARGO_HOME: cargoHome }, encoding: "utf8" }
    )
    assert.equal(result.status, 7, result.stderr)
    const output = JSON.parse(result.stdout.trim())
    assert.deepEqual(output.args, [
      "check",
      "--no-default-features",
      "--bin",
      "codeg-mcp",
    ])
    assert.equal(
      output.cwd,
      new URL("..", import.meta.url).pathname.replace(/\/$/, "")
    )
    assert.equal(output.target, join(output.cwd, "target"))
  }
)
