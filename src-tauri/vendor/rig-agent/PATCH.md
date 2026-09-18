# rig-agent 0.42.0 patch

Vendored from crates.io `rig-agent` 0.42.0. Do not edit `~/.cargo` cache.

Adds one hook: **committed-messages checkpoint**.

- Types: `CommittedMessages`, `CommittedMessagesNext`, `CommittedMessagesAction`
- `AgentHook::on_messages_committed` defaults to `Continue` so existing impls keep compiling
- Plumbed through `for_each_boxed_hook_event!` and `stack_first_non_continue!` like `on_model_turn_finished`
- Call site is only shared `drive_agent` after `run.next_step()`:
  - `Ok(step)`: fire with `CallModel` / `CallTools` / `Done` **before** dispatching the step
  - `Err(_)`: fire with `Failed`; a hook `Stop` replaces the original error

Does not add `StepEventKind::MessagesCommitted`. Does not change memory policies, `CompactingMemory`, tool concurrency, or `max_turns`.
