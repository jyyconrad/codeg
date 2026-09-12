---
name: using-plan-explore
description: Use when deciding whether to enter plan mode, spawn the explore subagent, or track work with update_plan; also when a coding task is ambiguous, spans many files, or you are about to edit without a written plan.
---

# Using plan, explore, and update_plan

Pick the tool by what you need. Do not enter plan mode or spawn explore by habit.

## Choose

| Situation | Use |
| --- | --- |
| Approach is ambiguous, high-impact, or live evidence just invalidated the design | `enter_plan_mode` |
| Broad search, unknown paths, "how does X work in this repo" | `subagent` (explore) |
| Known path, short read | `read_file` |
| Implementing a settled plan; need a checklist | `update_plan` |
| One-file fix, rename, or a change that follows an existing pattern | stay in code |

## Plan mode

Call `enter_plan_mode` with a short `reason`. The user must allow the call. After it succeeds, the **next** turn is plan mode: no `write_file`, `edit_file`, `bash`, or `update_plan`. Mid-implementation is allowed if the current approach is wrong.

In plan mode:

1. Research (you may still call `subagent`).
2. Ask the user only for decisions that change the approach (`ask_user_question`).
3. Write the full Markdown plan with `write_plan`.
4. Call `exit_plan_mode` and wait for Approve, request-changes, or Abandon.

If the user says "just implement" while still in plan mode, revise the plan. Do not invent source edits.

`update_plan` does **not** enter or exit plan mode. It is a code-mode checklist only.

## Explore (`subagent`)

Use `subagent` with `subagent_type` `explore` (default) for slow or multi-file investigation. Optional `thoroughness`: `quick`, `medium`, `very_thorough`.

The tool returns `started` immediately. That is not the report. A later user message starts with `Explore report ready.` and has `path:` and `summary:` only. Call `read_file` on `path` before planning or implementing. The summary is not a substitute for the report.

Do not poll. Do not start a second subagent while one is running.

## `update_plan` (session todo)

Code mode only. Replace the checklist with `pending` / `in_progress` / `completed` entries. Keep exactly one `in_progress`. Track an **already approved** plan; do not use it to design the approach.

## Common mistakes

| Mistake | Instead |
| --- | --- |
| Edit a multi-approach feature immediately | `enter_plan_mode` |
| Grep the whole tree in the main session | `subagent` |
| Treat `started` or the explore summary as enough | `read_file` the report path |
| Call `update_plan` to "start planning" | `enter_plan_mode` |
| Call `subagent` for one known file | `read_file` |
