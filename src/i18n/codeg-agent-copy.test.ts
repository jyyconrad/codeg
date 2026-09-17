import { readFileSync } from "node:fs"
import { resolve } from "node:path"
import { describe, expect, it } from "vitest"

import ar from "./messages/ar.json"
import de from "./messages/de.json"
import en from "./messages/en.json"
import es from "./messages/es.json"
import fr from "./messages/fr.json"
import ja from "./messages/ja.json"
import ko from "./messages/ko.json"
import pt from "./messages/pt.json"
import zhCN from "./messages/zh-CN.json"
import zhTW from "./messages/zh-TW.json"

const ALL_LOCALES = [
  ["en", en],
  ["ar", ar],
  ["de", de],
  ["es", es],
  ["fr", fr],
  ["ja", ja],
  ["ko", ko],
  ["pt", pt],
  ["zh-CN", zhCN],
  ["zh-TW", zhTW],
] as const

const REQUIRED_KEYS = [
  "inProcess",
  "experimental",
  "noSandbox",
  "recovery",
  "contextWindowsHint",
  "protocol",
  "modelSelectorHint",
  "usageLastRequest",
  "envWindowsHint",
  "configCardTitle",
  "bindProvider",
  "addModelProvider",
  "bindProviderFix",
  "addProviderFix",
  "editWindowFix",
  "modelReadOnly",
  "contextWindow",
  "maxOutput",
  "systemPrompt",
  "systemPromptHint",
  "compactPrompt",
  "compactPromptHint",
  "save",
  "envAdvanced",
  "envAdvancedHint",
  "boundCredentials",
  "emptyUsesBuiltin",
  "openDedicatedSettings",
  "compactSoftPercent",
  "compactRecentTurns",
  "maxTurns",
] as const

const SETTINGS_REQUIRED_KEYS = [
  "contextTitle",
  "contextDescription",
  "injectConstraints",
  "injectConstraintsHint",
  "injectAgentsMd",
  "injectClaudeMd",
  "injectTree",
  "injectTreeHint",
] as const

type CodegAgentCopy = Record<(typeof REQUIRED_KEYS)[number], string>

function codegAgentCopy(messages: unknown): CodegAgentCopy {
  const block = (
    messages as { AcpAgentSettings: { codegAgent: CodegAgentCopy } }
  ).AcpAgentSettings.codegAgent
  return block
}

describe("Codeg Agent experimental copy", () => {
  it("covers ten locales", () => {
    expect(ALL_LOCALES).toHaveLength(10)
  })

  it.each(ALL_LOCALES)(
    "%s has the experimental Codeg Agent keys",
    (_locale, messages) => {
      const block = codegAgentCopy(messages)
      for (const key of REQUIRED_KEYS) {
        expect(block[key], key).toEqual(expect.any(String))
        expect(block[key].trim().length, key).toBeGreaterThan(0)
      }
    }
  )

  it("English names Chat Completions and no OS sandbox", () => {
    const block = codegAgentCopy(en)
    expect(block.protocol).toMatch(/Chat Completions/)
    expect(block.noSandbox.toLowerCase()).toMatch(/sandbox/)
    expect(block.experimental.toLowerCase()).toMatch(/experimental/)
    expect(block.usageLastRequest.toLowerCase()).toMatch(/last model request/)
    expect(block.inProcess.toLowerCase()).toMatch(
      /bind a model provider and save/
    )
    expect(block.inProcess).not.toMatch(/CODEG_AGENT_CONTEXT_WINDOWS/)
    expect(block.openDedicatedSettings.toLowerCase()).toMatch(/built-in agent/)
  })

  it.each(ALL_LOCALES)(
    "%s protocol names Chat Completions",
    (_locale, messages) => {
      expect(codegAgentCopy(messages).protocol).toMatch(/Chat Completions/)
    }
  )
})

describe("Codeg Agent settings context copy", () => {
  it.each(ALL_LOCALES)("%s has context-inject keys", (_locale, messages) => {
    const block = (
      messages as {
        CodegAgentSettings: Record<
          (typeof SETTINGS_REQUIRED_KEYS)[number],
          string
        >
      }
    ).CodegAgentSettings
    for (const key of SETTINGS_REQUIRED_KEYS) {
      expect(block[key], key).toEqual(expect.any(String))
      expect(block[key].trim().length, key).toBeGreaterThan(0)
    }
    expect(block.injectAgentsMd).toBe("AGENTS.md")
    expect(block.injectClaudeMd).toBe("CLAUDE.md")
  })
})

describe("Codeg Agent README roster", () => {
  const readmes = [
    ["README.md", /sixteen agents come built in/],
    ["docs/readme/README.zh-CN.md", /内置十六个智能体/],
    ["docs/readme/README.zh-TW.md", /內建十六個智慧體/],
    ["docs/readme/README.ja.md", /16 種を内蔵/],
    ["docs/readme/README.ko.md", /열여섯 개의 에이전트가 기본 내장/],
    ["docs/readme/README.es.md", /dieciséis agentes integrados/],
    ["docs/readme/README.de.md", /sechzehn Agenten sind eingebaut/],
    ["docs/readme/README.fr.md", /seize agents sont intégrés/],
    ["docs/readme/README.pt.md", /dezesseis agentes já vêm integrados/],
    ["docs/readme/README.ar.md", /ستة عشر وكيلاً مدمجًا/],
  ] as const

  it.each(readmes)(
    "%s lists sixteen built-ins including Codeg Agent",
    (file, count) => {
      const text = readFileSync(resolve(process.cwd(), file), "utf8")
      expect(text).toMatch(count)
      expect(text).toContain("Codeg Agent")
    }
  )
})
