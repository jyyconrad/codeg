import { describe, expect, it } from "vitest"

import {
  effectiveWikiPrompt,
  filterWikiVaultNoise,
  groupWikiMemoryNotesByProject,
  latestFailedWikiSynthesizeJob,
  normalizeWikiList,
  normalizeWikiSettings,
  parseWikilink,
  resolveWikiNotePath,
  sanitizeWikilinkPath,
  splitWikiMarkdown,
  vaultNodesUnderPrefix,
  wikiProjectDisplayTitle,
  wikiSettingsPayload,
  wikiSourceCardTitle,
  wikiSynthesizeEnabled,
  type WikiJob,
  type WikiMemoryNote,
  type WikiProjectBinding,
  type WikiVaultTreeNode,
} from "./wiki-types"

const sampleTree: WikiVaultTreeNode[] = [
  { path: "index.md", name: "index.md", is_dir: false },
  {
    path: "work",
    name: "work",
    is_dir: true,
    children: [
      { path: "work/index.md", name: "index.md", is_dir: false },
      {
        path: "work/projects",
        name: "projects",
        is_dir: true,
        children: [
          {
            path: "work/projects/alpha.md",
            name: "alpha.md",
            is_dir: false,
          },
        ],
      },
    ],
  },
  {
    path: "raw",
    name: "raw",
    is_dir: true,
    children: [
      { path: "raw/sessions", name: "sessions", is_dir: true, children: [] },
    ],
  },
  {
    path: ".obsidian",
    name: ".obsidian",
    is_dir: true,
    children: [{ path: ".obsidian/app.json", name: "app.json", is_dir: false }],
  },
]

describe("vaultNodesUnderPrefix", () => {
  it("returns the full tree when prefix is empty", () => {
    expect(vaultNodesUnderPrefix(sampleTree, "")).toEqual(sampleTree)
    expect(vaultNodesUnderPrefix(sampleTree, "/")).toEqual(sampleTree)
  })

  it("unwraps a matching directory and keeps nested notes", () => {
    const work = vaultNodesUnderPrefix(sampleTree, "work")
    expect(work.map((node) => node.path)).toEqual([
      "work/index.md",
      "work/projects",
    ])
    expect(work[1]?.children?.[0]?.path).toBe("work/projects/alpha.md")
  })
})

describe("filterWikiVaultNoise", () => {
  it("hides raw and .obsidian by default", () => {
    const filtered = filterWikiVaultNoise(sampleTree)
    expect(filtered.map((node) => node.path)).toEqual(["index.md", "work"])
  })

  it("keeps raw when includeRaw is true", () => {
    const filtered = filterWikiVaultNoise(sampleTree, { includeRaw: true })
    expect(filtered.map((node) => node.path)).toEqual([
      "index.md",
      "work",
      "raw",
    ])
  })
})

describe("wiki settings prompts", () => {
  it("saves null when the edited prompt matches the builtin skill", () => {
    const builtin = "# wiki-turn-summary\nnever rewrite"
    const view = normalizeWikiSettings({
      turn_summary: { model_id: null, prompt: builtin },
      session_rollup: { model_id: null, prompt: "  " },
      synthesize: { enabled: true, model_id: null, prompt: "  " },
      turn_summary_builtin_prompt: builtin,
      session_rollup_builtin_prompt: "# wiki-session-rollup",
      synthesize_builtin_prompt: "# wiki-synthesize",
    })
    const payload = wikiSettingsPayload(view)
    expect(payload.turn_summary.prompt).toBeNull()
    expect(payload.session_rollup.prompt).toBeNull()
    expect(payload.synthesize.prompt).toBeNull()
    expect(payload).not.toHaveProperty("ingest")
    expect(payload).not.toHaveProperty("compile")
  })

  it("keeps a custom full prompt", () => {
    const view = normalizeWikiSettings({
      turn_summary: { model_id: null, prompt: "# custom turn" },
      turn_summary_builtin_prompt: "# builtin turn",
    })
    expect(wikiSettingsPayload(view).turn_summary.prompt).toBe("# custom turn")
    expect(effectiveWikiPrompt(null, "# builtin turn")).toBe("# builtin turn")
    expect(effectiveWikiPrompt("# custom", "# builtin turn")).toBe("# custom")
  })

  it("does not copy retired ingest/compile prompt text into the new slots", () => {
    const view = normalizeWikiSettings({
      ingest: { model_id: "old-ingest", prompt: "old ingest prompt" },
      compile: {
        enabled: false,
        model_id: "old-compile",
        prompt: "old compile prompt",
      },
    })
    expect(view.turn_summary.prompt).toBeNull()
    expect(view.session_rollup.prompt).toBeNull()
    expect(view.synthesize.prompt).toBeNull()
    expect(view.turn_summary.model_id).toBeNull()
    expect(view.session_rollup.model_id).toBeNull()
    expect(view.synthesize.model_id).toBeNull()
    expect(view.synthesize.enabled).toBe(false)
  })

  it("prefers synthesize.enabled over the legacy compile.enabled fallback", () => {
    const view = normalizeWikiSettings({
      compile: { enabled: false, model_id: null, prompt: null },
      synthesize: { enabled: true, model_id: null, prompt: null },
    })
    expect(view.synthesize.enabled).toBe(true)
    expect(wikiSynthesizeEnabled(view)).toBe(true)
  })
})

const sampleProject = (id: string, name: string): WikiProjectBinding => ({
  id,
  vault_id: "v",
  db_instance_id: "d",
  root_folder_id: 1,
  root_folder_name: name,
})

describe("wiki memory notes", () => {
  it("groups turn/session notes by project_binding_ids and leaves the rest ungrouped", () => {
    const alpha = sampleProject("p-alpha", "alpha")
    const beta = sampleProject("p-beta", "beta")
    const notes: WikiMemoryNote[] = [
      {
        rel: "work/turns/t1.md",
        page_type: "turn-summary",
        title: "Fixed auth timeout",
        summary: "Raised the token refresh window.",
        occurred_at: "2026-09-13T10:00:00Z",
        project_binding_ids: ["p-alpha"],
      },
      {
        rel: "work/sessions/c1.md",
        page_type: "session-summary",
        title: "Shipped the login flow",
        summary: "Completed the session after the timeout fix.",
        occurred_at: "2026-09-13T12:00:00Z",
        project_binding_ids: ["p-alpha", "p-beta"],
      },
      {
        rel: "work/turns/t2.md",
        page_type: "turn-summary",
        title: "Unbound turn",
        summary: "No project yaml.",
        project_binding_ids: [],
      },
      {
        rel: "sources/s1.md",
        page_type: "source",
        title: "ACP turn: should not render",
        project_binding_ids: ["p-alpha"],
      },
    ]
    const grouped = groupWikiMemoryNotesByProject(notes, [alpha, beta])
    expect(grouped.groups.map((g) => g.project.id)).toEqual([
      "p-alpha",
      "p-beta",
    ])
    expect(grouped.groups[0]?.notes.map((n) => n.title)).toEqual([
      "Shipped the login flow",
      "Fixed auth timeout",
    ])
    expect(grouped.groups[1]?.notes.map((n) => n.title)).toEqual([
      "Shipped the login flow",
    ])
    expect(grouped.ungrouped.map((n) => n.title)).toEqual(["Unbound turn"])
  })

  it("stays empty when there are no memory notes even if projects exist", () => {
    const grouped = groupWikiMemoryNotesByProject(
      [],
      [sampleProject("p1", "switchgear")]
    )
    expect(grouped.groups).toEqual([])
    expect(grouped.ungrouped).toEqual([])
  })

  it("reads { items } or a bare array from wiki_list_memory_notes", () => {
    const note: WikiMemoryNote = {
      rel: "work/turns/t1.md",
      page_type: "turn-summary",
      title: "Did the work",
    }
    expect(normalizeWikiList<WikiMemoryNote>({ items: [note] }).items).toEqual([
      note,
    ])
    expect(normalizeWikiList<WikiMemoryNote>([note]).items).toEqual([note])
  })
})

describe("latestFailedWikiSynthesizeJob", () => {
  it("ignores retired compile errors such as segment candidate overflow", () => {
    const jobs: WikiJob[] = [
      {
        id: "old-compile",
        kind: "compile",
        status: "failed",
        error_message: "segment s16 returned more than 5 candidates",
        finished_at: "2026-09-13T15:00:00Z",
      },
      {
        id: "older-synth",
        kind: "wiki_synthesize",
        status: "failed",
        error_message: "older synthesize failure",
        finished_at: "2026-09-12T03:00:00Z",
      },
      {
        id: "latest-synth",
        kind: "wiki_synthesize",
        status: "failed",
        error_message: "latest synthesize failure",
        finished_at: "2026-09-13T04:00:00Z",
      },
    ]
    const latest = latestFailedWikiSynthesizeJob(jobs)
    expect(latest?.id).toBe("latest-synth")
    expect(latestFailedWikiSynthesizeJob(jobs.slice(0, 1))).toBeNull()
  })
})

describe("wiki display titles", () => {
  it("prefers the folder name over Project {id}", () => {
    expect(
      wikiProjectDisplayTitle({
        id: "p1",
        vault_id: "v",
        db_instance_id: "d",
        root_folder_id: 2,
        root_folder_name: "switchgear",
      })
    ).toBe("switchgear")
    expect(
      wikiProjectDisplayTitle({
        id: "p1",
        vault_id: "v",
        db_instance_id: "d",
        root_folder_id: 2,
      })
    ).toBe("Project 2")
  })

  it("uses conversation title before the source id", () => {
    expect(
      wikiSourceCardTitle({
        id: "bb4c8465-288e-4d79-8385-d3e7d5a94e3b",
        source_title: "方案 C 样本流诊断窗分批改造",
      })
    ).toBe("方案 C 样本流诊断窗分批改造")
    expect(
      wikiSourceCardTitle({
        id: "bb4c8465-288e-4d79-8385-d3e7d5a94e3b",
      })
    ).toBe("bb4c8465-288e-4d79-8385-d3e7d5a94e3b")
  })
})

describe("wikilink parse", () => {
  it("parses path and aliased label", () => {
    expect(parseWikilink("work/index|Work")).toEqual({
      target: "work/index",
      label: "Work",
    })
    expect(parseWikilink("capabilities/index")).toEqual({
      target: "capabilities/index",
      label: "capabilities/index",
    })
  })

  it("strips .md and heading fragments", () => {
    expect(sanitizeWikilinkPath("work/index.md#Top")).toBe("work/index")
  })

  it("ignores http(s) and parent-dir escapes", () => {
    expect(sanitizeWikilinkPath("https://example.com/note")).toBeNull()
    expect(sanitizeWikilinkPath("http://example.com/note")).toBeNull()
    expect(sanitizeWikilinkPath("../secrets")).toBeNull()
    expect(sanitizeWikilinkPath("/etc/passwd")).toBeNull()
  })

  it("resolves a vault-relative markdown path", () => {
    expect(resolveWikiNotePath("work/index")).toBe("work/index.md")
    expect(resolveWikiNotePath("https://example.com")).toBeNull()
  })

  it("splits markdown into text and wikilink parts", () => {
    const parts = splitWikiMarkdown(
      "See [[work/index|Work]] and [[https://x.test]]."
    )
    expect(parts).toEqual([
      { kind: "text", text: "See " },
      {
        kind: "wikilink",
        raw: "[[work/index|Work]]",
        label: "Work",
        target: "work/index",
      },
      { kind: "text", text: " and " },
      {
        kind: "wikilink",
        raw: "[[https://x.test]]",
        label: "https://x.test",
        target: null,
      },
      { kind: "text", text: "." },
    ])
  })
})
