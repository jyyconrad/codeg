---
name: wiki-compile
description: Use when compiling wiki sources into personal work notes and capability notes. Process only the host manifest.
disable-model-invocation: true
---

# Wiki compile

You compile **personal work notes** and **capability notes** from frozen wiki sources. Process **only** the source versions and segments in the **host manifest**. Raw evidence is immutable. Compiled pages are separate. `log.md` is an operations log, not a queue; do not infer progress from it.

WikiWorker loads this skill on purpose. You do not scan all user skills. You do not finalize the vault. **Step 4 finalize is HOST**: you return staged page proposals plus an explicit processed-input manifest. The host validates and commits files, indexes, logs, and progress.

## Hard never

- Never read or invent sources outside the host manifest.
- Never modify `raw/`. Never treat `log.md` or file mtime as the work queue.
- Never merge on title alone. **related≠same**. Same title in two projects is two identities.
- Never assign `project_id`. The host owns project mapping.
- Never add a fact, number, or outcome without a source locator.
- Never attribute agent or team actions to the user without `personal_role` evidence.
- Never promote reference-only material to personal practice. Reference ⇒ `evidence_level: knowledge_only`.
- Never write proficiency ranks (“expert”, “senior”) or add skill from import counts.
- Never overwrite user-authored regions or drop old evidence to hide a conflict.
- Never mark a decision `adopted` because the model suggested it.
- Never create a project named `chat` for unattributed material.
- Never mint `codeg_note_id` / `source_id`; echo host ids. You may suggest titles and slugs only.
- Max **5** durable knowledge candidates per segment. “Nothing to persist” is valid.

## Grounding (WeKnora)

Static rules stay at the front of this skill. Host input is appended last. Do not invent from filenames, folder names, or host metadata when the segment text is empty or non-substantive.

- If a segment has no extractable claims, return **no candidates** for it. That is success.
- **related ≠ same.** Overlapping topic, shared vocabulary, or similar titles are not identity. Merge only when kind, work scope, and claim identity match.
- Reuse the host `codeg_note_id` when the same identity still exists. Do not mint a parallel page for the same claim.
- Aliases mean **the exact same thing** (abbreviation, translation, short/full name). Parent categories and related techniques are not aliases.
- Every fact in a page body needs a locator next to the claim. A page-level `sources` list is not enough.
- Do not guess a topic from a scanner-style filename or a path. Content is the only source of claims.
- Keep three voices distinct: **the source said**, **this work used**, **personal practice showed**.

## Four steps (WeKnora discipline)

| Step | You do | Host does |
| --- | --- | --- |
| 1. Candidates | Extract claims with locators and `evidence_type` | Supplies the frozen manifest only |
| 2. Match | Classify **same** / **related** / **contradictory** / **new** | Provides index + stable ids; `project_id` is host-owned |
| 3. Merge | Staged page proposals; preserve contradictions and user-authored regions | Writes staging; rejects facts without locators |
| 4. Finalize | Return proposals + processed-input manifest | **HOST** validates, commits, rebuilds indexes/logs |

Do not skip to page prose before candidates have locators. Do not call a related page the same page.

### Step 1 — candidates

From each manifest segment, extract at most **5** durable candidates. Kinds:

- `work_context` — project/area background, role, constraints
- `decision` — options, choice, rationale (often still `proposed`)
- `outcome` — delivered result with evidence
- `method` / `concept` — reusable procedure or named idea
- `capability_evidence` — application, result, or reflection tied to a skill

Every candidate **must** include:

- `locator`: `source_id`, `segment_id`, pointer (PDF physical page / DOCX heading+para / MD heading+para / ACP region)
- `evidence_type`: `reference` | `application` | `result` | `reflection`
- `actor`: `user` | `agent` | `team` | `unspecified`
- `claim`: one checkable sentence copied from evidence, not a vibe

If the segment is a spec, FAQ, or chat with no durable work/capability content, return **no candidates**. That is success.

### Step 2 — match (related≠same)

Compare each candidate to host-provided existing pages.

- **same**: same kind, same work scope, same claim identity (stable `codeg_note_id` / case id if host gave one). Update that page.
- **related**: overlapping topic, different project, different meaning, or insufficient proof of identity. Link; do **not** merge.
- **contradictory**: same identity, conflicting claims. Keep both with dates, scope, and locators. Do not let the newer source silently win.
- **new**: no justified identity. Propose a new page; host allocates path and `codeg_note_id`.

Never merge solely because titles, slugs, or folder names match. Homonyms stay distinct. When unsure, mark `needs_review` and keep them separate.

### Step 3 — merge (staged proposals)

Emit full page proposals for staging. Rules:

- New facts require locators next to the claim in the body, not only a page-level `sources` list.
- Preserve contradictions in prose (when, where each applies, both sources).
- Preserve user-authored regions the host marked. Do not rewrite them.
- Do not delete old evidence because a new source is silent about it.
- Formal replacement uses `status: superseded` plus a link to the replacement page — not “overwrite the old claim”.
- YAML is **flat**. Wikilinks in YAML are **quoted** strings. Lists of links are lists of quoted strings. Vault-relative paths, no `.md`, `/` separators: `"[[work/projects/codeg-<id>|Codeg]]"`.

### Step 4 — finalize is HOST

You stop after JSON + staged proposal bodies. You do not:

- commit vault files, rebuild `index.md` generation zones, or append `log.md`
- register consumption, resolve hash conflicts, or mark jobs succeeded

Return `processed_inputs` covering **every** manifest segment you were given, including those with nothing to persist.

## Work wiki page contract

Create personal work notes around **projects, responsibilities, decisions, actions, and outcomes**. Distinguish **proposals**, **reported actions**, and **results**.

| `type` | Must answer | When evidence is thin |
| --- | --- | --- |
| `project` | Background, goals, personal role, progress, decisions, outcomes, open problems, related capabilities | Unknown role/result → “unspecified”; do not infer goals from a folder name |
| `area` | Long-running responsibility, standards, related projects, reusable methods | You may **suggest** an existing area; do not invent a new area identity unless the host asked |
| `work-record` | Problem, context, actions taken, observed results, personal contribution, sources | ACP may record **agent** actions; that is not “user completed it” |
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

These are not a skill score. `user_confirmed` does not replace a test or third-party review. v1 only **cites** existing verification; you do not run code or call delivery systems.

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
- `source` reading pages are compile output (derived). Raw dumps stay host-owned.

Keep three voices distinct in prose: **the source said**, **this work used**, **personal practice showed**.

## YAML keys (flat; quoted wikilinks)

Common (every compiled note):

```yaml
title: Interface design
summary: One-line derived summary; not a substitute for raw.
type: capability
tags:
  - type/capability
aliases: []
date: 2026-09-12
updated: 2026-09-12
status: draft
projects: []
areas: []
sources:
  - "[[sources/11111111-1111-4111-8111-111111111111]]"
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

Return **only** JSON. Proposal bodies are strings (markdown with YAML frontmatter) or omitted when `nothing_to_persist` is true.

```json
{
  "schema": "codeg.wiki.compile.v1",
  "compile_contract_version": "<echo host>",
  "processed_inputs": [
    {
      "source_id": "<host>",
      "raw_hash": "<host>",
      "annotation_revision": 1,
      "segment_ids": ["<host>"]
    }
  ],
  "candidates": [
    {
      "candidate_id": "c1",
      "kind": "method",
      "title": "Idempotent retry with idempotency keys",
      "claim": "The spec requires an idempotency key on retried POSTs.",
      "evidence_type": "reference",
      "actor": "unspecified",
      "personal_role_in_source": null,
      "locator": {
        "source_id": "<host>",
        "segment_id": "<host>",
        "pointer": "§3.2 / page 4 / para 2"
      }
    }
  ],
  "matches": [
    {
      "candidate_id": "c1",
      "relation": "related",
      "existing_note_id": null,
      "project_id": null,
      "reason": "Same topic as an existing method page but different system; related≠same."
    }
  ],
  "page_proposals": [
    {
      "op": "create",
      "type": "method",
      "title": "Idempotent retry",
      "codeg_note_id": null,
      "before_hash": null,
      "body": "---\ntitle: Idempotent retry\ntype: method\n...\n"
    }
  ],
  "nothing_to_persist": false,
  "needs_review": [],
  "warnings": []
}
```

`processed_inputs` must list every manifest source/segment in this call, including no-ops. The host uses that list — not `log.md` — to record consumption.

`page_proposals[].op`: `create` | `update` | `supersede`. Updates must echo `codeg_note_id` and the `before_hash` the host supplied.

## Attribution

- No `personal_role` → do not say the user did it.
- Agent tool traces → actor `agent`. Useful as context, not personal practice.
- Team report with unknown personal role → team result, not a personal win; methods may still be `knowledge_only`.
- First-person in a document does not prove the Codeg user is the author unless annotations say so.

## Budget

- Only manifest segments. Default ≤8000 characters of segment text per batch unless the host sent less.
- ≤5 durable candidates per segment.
- Do not silently drop a segment and mark the source complete. If a segment yields nothing, record it in `processed_inputs` with no candidates.

## Failure

Invalid locators, title-only merges, reference material labeled as practice, or missing `processed_inputs` coverage are failed compile output. Prefer `nothing_to_persist: true` over invented pages.
