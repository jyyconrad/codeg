---
name: wiki-synthesize
description: Use when WikiWorker is updating personal work wiki and capability wiki from memory notes (turn/session pages), not from raw segment candidates.
disable-model-invocation: true
---

# Wiki synthesize

You update **personal work notes** and **capability notes** from host-listed **memory notes** (`work/turns/`, `work/sessions/`) plus the existing page index. Compiled pages are separate from memory pages. `log.md` is an operations log, not a queue.

WikiWorker loads this skill on purpose. You do not scan all user skills. **Finalize is HOST**: return staged page proposals plus `processed_inputs`. The host validates and commits files, indexes, logs, and progress. No bash, no MCP, no subagent.

This is **not** a candidate-extraction job. Do not return a `candidates` array. Do not apply per-segment candidate quotas. Do not run a four-step candidate pipeline. Omit `candidates`.

## Inputs

The host lists memory-note `rel`s (and content hashes) and an existing page index. Read those notes with `read_file` / `grep` / `glob`. You may read index paths, `work/`, `capabilities/`, `knowledge/`, `sources/`, and `AGENTS.md` the host listed.

**Project folders** may appear as optional read roots. Not reading them is success. Do not dump a repo tree into context or into a page. Do not write into a project tree. Do not submit project files as wiki pages.

Do not invent sources outside the host list. Do not modify `raw/`. Do not treat `log.md` or mtime as the work queue. Import documents are out of this job unless the host listed a memory note.

## Hard never

- Never merge on title alone. **related≠same**. Same title in two projects is two identities.
- Never assign `project_id`. The host owns project mapping.
- Never add a fact, number, or outcome without a locator into a memory note (or a host-listed page).
- Never attribute agent or team actions to the user without `personal_role` evidence.
- Never promote reference-only material to personal practice. Reference ⇒ `evidence_level: knowledge_only`.
- Never write proficiency ranks (“expert”, “senior”) or add skill from import counts.
- Never overwrite user-authored regions or drop old evidence to hide a conflict.
- Never mark a decision `adopted` because the model suggested it.
- Never create a project named `chat` for unattributed material.
- Never mint `codeg_note_id` / `source_id`; echo host ids. You may suggest titles and slugs only.
- Never require or emit a `candidates` array.
- “Nothing to persist” is valid when memory notes add no durable work/capability content.
- Each compiled leaf page should be long enough to keep locators and evidence, and no longer. Do not pad. The host warns on very long pages and rejects only runaway dumps.

## Identity (related≠same)

Compare memory claims to host-provided existing pages.

- **same**: same kind, same work scope, same claim identity (stable `codeg_note_id` / case id if host gave one). Update that page.
- **related**: overlapping topic, different project, different meaning, or insufficient proof of identity. Link; do **not** merge.
- **contradictory**: same identity, conflicting claims. Keep both with dates, scope, and locators. Do not let the newer note silently win.
- **new**: no justified identity. Propose a new page; host allocates path and `codeg_note_id`.

Never merge solely because titles, slugs, or folder names match. Homonyms stay distinct. When unsure, mark `needs_review` and keep them separate.

Aliases mean **the exact same thing** (abbreviation, translation, short/full name). Parent categories and related techniques are not aliases.

Reuse the host `codeg_note_id` when the same identity still exists. Do not mint a parallel page for the same claim.

Keep three voices distinct: **the source said**, **this work used**, **personal practice showed**.

## Staged proposals

Emit full page proposals for staging:

- New facts need locators next to the claim in the body, not only a page-level `sources` list.
- Preserve contradictions in prose (when, where each applies, both sources).
- Preserve user-authored regions the host marked. Do not rewrite them.
- Do not delete old evidence because a new memory note is silent about it.
- Formal replacement uses `status: superseded` plus a link to the replacement page.
- YAML is **flat**. Wikilinks in YAML are **quoted** strings. Lists of links are lists of quoted strings. Vault-relative paths, no `.md`, `/` separators: `"[[work/projects/codeg-<id>|Codeg]]"`.
- Proposal `body` is **full markdown WITH yaml front matter** for these compiled pages.

You stop after JSON + staged proposal bodies. You do not commit vault files, rebuild `index.md` generation zones, append `log.md`, register consumption, or resolve hash conflicts.

## Work wiki page contract

Create personal work notes around **projects, responsibilities, decisions, actions, and outcomes**. Distinguish **proposals**, **reported actions**, and **results**.

| `type` | Must answer | When evidence is thin |
| --- | --- | --- |
| `project` | Background, goals, personal role, progress, decisions, outcomes, open problems, related capabilities | Unknown role/result → “unspecified”; do not infer goals from a folder name |
| `area` | Long-running responsibility, standards, related projects, reusable methods | You may **suggest** an existing area; do not invent a new area identity unless the host asked |
| `work-record` | Problem, context, actions taken, observed results, personal contribution, sources | ACP/memory may record **agent** actions; that is not “user completed it” |
| `decision` | Constraints, options, choice, rationale, impact, time scope, replacement | Suggestion only → `decision_state: proposed` |
| `outcome` | Artifact, personal contribution, result evidence, reusable residue | Plan or self-claim of “done” stays unverified; do not invent benefit metrics |

`decision_state`: `proposed` | `adopted` | `replaced`. This is **not** note `status`.

Note `status`: `draft` | `active` | `superseded` (describes the note, not the job).

A Q&A turn may update a method/source summary and produce **zero** outcomes. Do not mint empty trophy pages.

## Capability wiki page contract

Capability pages are named after **reusable work tasks** (e.g. “interface design”), never after a project name or a model name.

Required sections:

1. Scope — what it solves; when it does not apply
2. Methods and checkpoints — steps; link shared method/concept pages
3. Practice evidence — cases with actor, `personal_role`, `evidence_type`, `verification_status`, source, locator
4. Current boundary — observed limits, failures, counterexamples
5. Next practice — suggested exercise and observable check; **suggestion only**, not an automatic task

### evidence_type

| Type | May support | Must not conclude |
| --- | --- | --- |
| `reference` | A documented method exists; the document claims X | User read, understood, practiced, or mastered it |
| `application` | Some work used the method; name the actor | User did it alone or has stable mastery |
| `result` | A test/review/delivery happened in a stated environment | One case proves all cases; team win = personal win |
| `reflection` | User described tradeoffs, failures, or corrections | The reflection is externally verified |

### verification_status

`source_reported` | `user_confirmed` | `artifact_checked`

These are not a skill score. `user_confirmed` does not replace a test or third-party review. You cite existing verification; you do not run code or call delivery systems.

### evidence_level (capability pages)

- `knowledge_only` — reference material, no personal-role practice
- `practice_reported` — user-role application without a checkable result
- `practice_supported` — personal-role case with a concrete, checkable result (still recorded as source-reported unless verification says otherwise)

**Reference material alone establishes `knowledge_only`, never personal practice.** Agent-only or team-only cases may be cited; they do **not** raise the user’s evidence level.

Identical cases across many turns/files are **one** case. Dedup by host `note_id` / `case_id`. If identity is uncertain, mark `needs_review`; do not stack mastery.

Put evidence rows in the body (case, actor, personal_role, evidence_type, verification_status, source, locator). Do not nest that structure in YAML.

## Shared knowledge and sources

- `concept` — named rule/constraint
- `method` — inputs, steps, outputs, applicability bounds
- `entity` — only when necessary (system, product, org); not every proper noun
- `source` reading pages are derived. Raw dumps stay host-owned.

Do not emit `turn-summary` / `session-summary` pages from this job. Memory notes are inputs, not synthesize outputs.

## YAML keys (flat; quoted wikilinks)

Common (every compiled note):

```yaml
title: Interface design
summary: One-line derived summary; not a substitute for memory notes.
type: capability
tags:
  - type/capability
aliases: []
date: 2026-09-13
updated: 2026-09-13
status: draft
projects: []
areas: []
sources: []
capabilities: []
concepts: []
codeg_note_id: "22222222-2222-4222-8222-222222222222"
```

Also, when applicable:

- Capability: `evidence_level: knowledge_only` (or `practice_reported` / `practice_supported`)
- Decision: `decision_state: proposed` (or `adopted` / `replaced`)
- `type` enum: `project` / `area` / `work-record` / `decision` / `outcome` / `capability` / `concept` / `method` / `entity` / `source`
- Lists stay lists even for a single project. Unknown links: omit, do not invent.
- Do not emit dangling machine wikilinks. Host allocates new paths; use only ids/paths the host gave, or leave `new_page` proposals without a path.

`tags` must include `type/<type>`. Dates are ISO dates. Do not put job status on the note.

## Output JSON (host validates)

Return **only** JSON. Schema `codeg.wiki.synthesize.v1`. Proposal bodies are strings (markdown **with** YAML front matter) or omitted when `nothing_to_persist` is true. Do not include a `candidates` key.

```json
{
  "schema": "codeg.wiki.synthesize.v1",
  "processed_inputs": [
    {
      "rel": "work/turns/<host source_id>.md",
      "content_hash": "<host>"
    }
  ],
  "page_proposals": [
    {
      "op": "create",
      "type": "work-record",
      "title": "List API cursor pagination",
      "codeg_note_id": null,
      "before_hash": null,
      "body": "---\ntitle: List API cursor pagination\ntype: work-record\n...\n"
    }
  ],
  "nothing_to_persist": false,
  "warnings": [],
  "needs_review": []
}
```

`processed_inputs` must list **every** memory note `{ rel, content_hash }` the host listed, including nothing-to-persist. The host uses that list — not `log.md` — to record consumption.

`page_proposals[].op`: `create` | `update` | `supersede`. Updates must echo `codeg_note_id` and the `before_hash` the host supplied.

## Attribution

- No `personal_role` → do not say the user did it.
- Agent tool traces / turn memory of agent edits → actor `agent`. Useful as context, not personal practice.
- Team report with unknown personal role → team result, not a personal win; methods may still be `knowledge_only`.
- First-person in a document does not prove the Codeg user is the author unless annotations say so.

## Budget

- Cover durable claims in the listed memory notes. Do not force-read project trees.
- Leaf page body length is your decision. Keep evidence and locators; do not pad.
- Do not silently drop a listed memory note. If it yields nothing, still record it in `processed_inputs`.

## Failure

Title-only merges, reference material labeled as practice, a required `candidates` array, or missing `processed_inputs` coverage are failed output. Prefer `nothing_to_persist: true` over invented pages.
