//! Wiki 后台任务的用户指令：与可编辑 SKILL 职责分离，设置预览和运行共用模板。

use std::path::Path;

use serde_json::{json, Map, Value};

fn stage_task(stage: &str) -> &'static str {
    match stage {
        "turn_summary" => include_str!("prompts/turn-summary.md"),
        "session_rollup" => include_str!("prompts/session-rollup.md"),
        "synthesize" => include_str!("prompts/synthesize.md"),
        _ => "# 整理当前 Wiki 任务\n\n请依据本次材料完成整理。",
    }
}

fn render(
    stage: &str,
    task_context: &str,
    workspace: &str,
    sources: &str,
    wiki_context: &str,
    output_contract: &str,
) -> String {
    format!(
        include_str!("prompts/task.md"),
        task = stage_task(stage),
        task_context = task_context,
        workspace = workspace,
        sources = sources,
        wiki_context = wiki_context,
        output_contract = output_contract,
    )
}

pub fn builtin_task_template(stage: &str) -> String {
    render(
        stage,
        "{{任务标识、来源或会话身份、发生时间、当前批次和模型轮次预算}}",
        "{{vault_abs：Wiki 绝对目录；staging_abs：本次暂存绝对目录；target_note：已有目标笔记位置（如有）}}",
        "{{材料清单：每项包含可读取的 read_path、来源关联 rel、source_ids 和材料类型 kind}}",
        "{{已有页面索引与可读地址、项目背景；没有时明确说明}}",
        "{{宿主按阶段附加完整 JSON 交付格式、身份要求、无内容结果及提案规则}}",
    )
}

/// Derive a usable address while keeping the original rel for attribution.
/// The filesystem tool still enforces canonical access and file existence.
fn read_path(vault: Option<&str>, rel: &str) -> Option<String> {
    if rel.is_empty() || rel.starts_with("source:") {
        return None;
    }
    let path = Path::new(rel);
    if path.is_absolute() {
        return Some(rel.to_string());
    }
    if !super::paths::is_safe_vault_relative(rel) {
        return None;
    }
    vault.map(|root| Path::new(root).join(path).to_string_lossy().into_owned())
}

pub(super) fn task_message(stage: &str, input: &Value, output_contract: &str) -> String {
    let vault = input.get("vault_abs").and_then(Value::as_str);
    let context: Map<String, Value> = [
        "schema",
        "job_id",
        "attempt",
        "source_id",
        "source_kind",
        "source_title",
        "conversation_id",
        "occurred_at",
        "batch_id",
        "max_turns",
        "source_warnings",
    ]
    .into_iter()
    .filter_map(|key| input.get(key).map(|value| (key.to_string(), value.clone())))
    .collect();
    let workspace = json!({
        "vault_abs": input.get("vault_abs"),
        "staging_abs": input.get("staging_abs"),
        "target_note": input.get("rel").and_then(Value::as_str).and_then(|rel| read_path(vault, rel)),
    });
    let sources: Vec<_> = input
        .get("source_references")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|source| {
            let rel = source.get("rel").and_then(Value::as_str).unwrap_or("");
            let is_turn = input
                .get("turn_rels")
                .and_then(Value::as_array)
                .is_some_and(|turns| turns.iter().any(|turn| turn.as_str() == Some(rel)));
            json!({
                "rel": rel,
                "read_path": read_path(vault, rel),
                "source_ids": source.get("source_ids").cloned().unwrap_or_else(|| json!([])),
                "kind": if is_turn { "turn_note" } else { "source_material" },
            })
        })
        .collect();
    let wiki_context = json!({
        "index": input.get("index").cloned().unwrap_or_else(|| json!([])),
        "project_metadata": input.get("project_metadata").cloned().unwrap_or_else(|| json!([])),
    });
    let pretty =
        |value: &Value| serde_json::to_string_pretty(value).expect("JSON value serializes");
    render(
        stage,
        &pretty(&Value::Object(context)),
        &pretty(&workspace),
        &pretty(&json!(sources)),
        &pretty(&wiki_context),
        output_contract,
    )
}
