//! 把模型的综合整理结果转换为可提交的Wiki页面。
//! compile提供来源路径和现有笔记索引；本模块分配新笔记身份、补来源链接，
//! 保留更新与替代关系，输出交由commit执行；不要求模型提供哈希或逐行证据。

use super::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct SynthesisOutput {
    schema: String,
    page_proposals: Vec<PageProposal>,
    #[serde(default)]
    warnings: Vec<String>,
}
#[derive(Deserialize)]
struct PageProposal {
    proposal_key: String,
    op: String,
    #[serde(rename = "type")]
    page_type: String,
    existing_note_id: Option<String>,
    title: String,
    summary: Option<String>,
    body: String,
    #[serde(default)]
    input_rels: Vec<String>,
    #[serde(default)]
    related_proposal_keys: Vec<String>,
    replacement_note_id: Option<String>,
    replacement_proposal_key: Option<String>,
}
pub(super) struct ValidatedOutput {
    pub proposals: Vec<StagedProposal>,
    pub processed: Vec<ProcessedInput>,
    pub warnings: Vec<String>,
}

pub(super) fn validate_output(
    value: Value,
    input_records: &[WikiInput],
    index: &[NoteIndexEntry],
) -> Result<ValidatedOutput, CompileError> {
    let out: SynthesisOutput = serde_json::from_value(value)
        .map_err(|e| CompileError::Validation(format!("v2 output: {e}")))?;
    if out.schema != SYNTHESIZE_CONTRACT_VERSION {
        return invalid("unsupported synthesis schema; update the custom prompt");
    }
    let inputs: HashMap<_, _> = input_records.iter().map(|r| (r.rel.as_str(), r)).collect();
    let mut identities = HashMap::new();
    let mut existing = HashMap::new();
    for item in index {
        if existing.insert(item.note_id.as_str(), item).is_some() {
            return Err(CompileError::Conflict(format!(
                "duplicate note identity {}",
                item.note_id
            )));
        }
    }
    let mut keys = HashSet::new();
    let mut paths = HashSet::new();
    for p in &out.page_proposals {
        if p.proposal_key.trim().is_empty() || !keys.insert(p.proposal_key.clone()) {
            return invalid("proposal keys must be nonempty and unique");
        }
        if !DOMAIN_TYPES.contains(&p.page_type.as_str()) {
            return invalid("type is not writable by synthesis");
        }
        if p.title.trim().is_empty() || p.title.contains(['\n', '\r']) || !substantive_body(&p.body)
        {
            return invalid(
                "proposal requires a title and substantive body without system frontmatter",
            );
        }
        check_leaf_body(&p.page_type, &p.body)?;
        let (id, rel, before) = match p.op.as_str() {
            "create" if p.existing_note_id.is_none() => {
                let id = Uuid::new_v4();
                (
                    id.to_string(),
                    crate::wiki::fs_policy::allocate_note_rel(&p.page_type, &p.title, &id)?,
                    String::new(),
                )
            }
            "update" | "supersede" => {
                let id = p.existing_note_id.as_deref().ok_or_else(|| {
                    CompileError::Validation("update requires existing_note_id".into())
                })?;
                let old = existing.get(id).ok_or_else(|| {
                    CompileError::Validation("existing_note_id is outside the Wiki index".into())
                })?;
                if old.page_type != p.page_type {
                    return invalid("existing note type cannot change");
                }
                (id.to_string(), old.rel.clone(), old.hash.clone())
            }
            _ => return invalid("unsupported proposal operation or create identity"),
        };
        if !paths.insert(rel.clone()) {
            return invalid("multiple proposals target the same note");
        }
        identities.insert(p.proposal_key.clone(), (id, rel, before));
    }
    let mut referenced = HashSet::new();
    let mut staged = Vec::new();
    let today = Utc::now().date_naive().to_string();
    for p in &out.page_proposals {
        let output_rel = &identities[&p.proposal_key].1;
        // Agent可仅返回正文；没有指定材料时关联本批来源，代码补归属而不要求回传读件证据。
        let mut declared: BTreeSet<&str> = p
            .input_rels
            .iter()
            .map(String::as_str)
            .filter(|rel| inputs.contains_key(rel))
            .collect();
        if declared.is_empty() {
            declared.extend(inputs.keys().copied());
        }
        let mut body = p.body.trim().to_owned();
        if !declared.is_empty() {
            body.push_str("\n\n## 来源\n");
            for rel in &declared {
                body.push_str(&format!(
                    "\n- [{}]({})",
                    rel.replace(['[', ']'], " "),
                    relative_note_path(output_rel, rel)
                ));
            }
        }
        for key in &p.related_proposal_keys {
            let (_, rel, _) = identities
                .get(key)
                .filter(|_| key != &p.proposal_key)
                .ok_or_else(|| CompileError::Validation("invalid related proposal key".into()))?;
            body.push_str(&format!(
                "\n\n[{}]({})",
                key,
                relative_note_path(output_rel, rel)
            ));
        }
        let mut replacement = None;
        if p.op == "supersede" {
            replacement = match (&p.replacement_note_id, &p.replacement_proposal_key) {
                (Some(id), None) => Some(
                    existing
                        .get(id.as_str())
                        .ok_or_else(|| {
                            CompileError::Validation(
                                "replacement note outside the Wiki index".into(),
                            )
                        })?
                        .rel
                        .clone(),
                ),
                (None, Some(key)) => Some(
                    identities
                        .get(key)
                        .filter(|_| key != &p.proposal_key)
                        .ok_or_else(|| {
                            CompileError::Validation("invalid replacement proposal key".into())
                        })?
                        .1
                        .clone(),
                ),
                _ => return invalid("supersede requires exactly one replacement target"),
            };
        } else if p.replacement_note_id.is_some() || p.replacement_proposal_key.is_some() {
            return invalid("replacement is only valid for supersede");
        }
        let (id, rel, before) = &identities[&p.proposal_key];
        if replacement.as_deref() == Some(rel) {
            return invalid("note cannot supersede itself");
        }
        let summary = p
            .summary
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| first_paragraph(&body));
        let mut yaml = serde_yaml::Mapping::new();
        for (key, val) in [
            ("title", p.title.as_str()),
            ("summary", summary.as_str()),
            ("type", p.page_type.as_str()),
            ("codeg_note_id", id.as_str()),
            ("date", today.as_str()),
            ("updated", today.as_str()),
            (
                "status",
                if replacement.is_some() {
                    "superseded"
                } else {
                    "active"
                },
            ),
            ("evidence_level", "knowledge_only"),
            ("verification_status", "source_reported"),
            ("personal_role", "unspecified"),
        ] {
            yaml.insert(key.into(), val.into());
        }
        yaml.insert(
            "tags".into(),
            serde_yaml::to_value(vec![format!("type/{}", p.page_type)]).map_err(serialization)?,
        );
        let source_ids: BTreeSet<String> = declared
            .iter()
            .flat_map(|r| inputs[*r].source_ids.iter().cloned())
            .collect();
        yaml.insert(
            "source_ids".into(),
            serde_yaml::to_value(source_ids).map_err(serialization)?,
        );
        yaml.insert(
            "sources".into(),
            serde_yaml::to_value(&declared).map_err(serialization)?,
        );
        if p.page_type == "decision" {
            yaml.insert("decision_state".into(), "proposed".into());
        }
        if let Some(replacement) = replacement {
            yaml.insert("superseded_by".into(), replacement.clone().into());
            // 替代关系保留旧正文，后续阅读才能看清观点变化，而不是被新结论覆盖。
            let old = existing[p.existing_note_id.as_deref().unwrap()];
            body = format!(
                "{}\n\nReplaced by [{}]({}).\n\n{}",
                old.body,
                replacement,
                relative_note_path(output_rel, &replacement),
                body
            );
        }
        let after = format!(
            "---\n{}---\n\n{CONTENT_START}\n{}\n{CONTENT_END}\n",
            serde_yaml::to_string(&yaml).map_err(serialization)?,
            body
        );
        check_leaf_body(&p.page_type, &after)?;
        staged.push(StagedProposal {
            rel: rel.clone(),
            page_type: p.page_type.clone(),
            before_hash: before.clone(),
            after,
            op: if p.op == "create" {
                "create".into()
            } else {
                "update".into()
            },
        });
        referenced.extend(declared.into_iter().map(str::to_owned));
    }
    let processed = input_records
        .iter()
        .map(|input| ProcessedInput {
            rel: input.rel.clone(),
            content_hash: input.content_hash.clone(),
            disposition: if referenced.contains(&input.rel) {
                "used"
            } else {
                "no_content"
            }
            .into(),
            source_ids: input.source_ids.clone(),
        })
        .collect();
    Ok(ValidatedOutput {
        proposals: staged,
        processed,
        warnings: out.warnings,
    })
}
fn serialization(error: serde_yaml::Error) -> CompileError {
    CompileError::Validation(error.to_string())
}
fn invalid<T>(reason: &str) -> Result<T, CompileError> {
    Err(CompileError::Validation(reason.into()))
}
fn substantive_body(body: &str) -> bool {
    let body = body.trim();
    !body.starts_with("---")
        && !body.contains("codeg-content:")
        && body.lines().any(|line| {
            let line = line.trim();
            !line.is_empty()
                && !line.starts_with('#')
                && !matches!(line, "暂无" | "暂无。" | "N/A" | "None")
                && !line.starts_with("[[")
                && !line.starts_with("- [")
                && line.chars().any(char::is_alphanumeric)
        })
}
fn first_paragraph(body: &str) -> String {
    body.lines()
        .find(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .unwrap_or_default()
        .chars()
        .take(240)
        .collect()
}

fn relative_note_path(from: &str, target: &str) -> String {
    let mut parent: Vec<_> = from.split('/').collect();
    parent.pop();
    let target: Vec<_> = target.split('/').collect();
    let common = parent
        .iter()
        .zip(&target)
        .take_while(|(a, b)| a == b)
        .count();
    let mut parts = vec!["..".to_owned(); parent.len() - common];
    parts.extend(
        target[common..]
            .iter()
            .map(|part| urlencoding::encode(part).into_owned()),
    );
    parts.join("/")
}
