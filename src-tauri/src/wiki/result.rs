//! 定义Wiki任务保存的笔记清单、来源关联和剩余素材。
//! 轮次/对话整理与综合批次生成这些结果，数据库服务持久化，读模型展示。
//! 素材摘要服务于重复归纳去重，输出摘要服务于提交恢复；两者都不是模型证据合同。

use serde::{Deserialize, Serialize};

/// 记录一次整理所关联的素材；content_hash只用于防止重复归纳，不限制Agent读取版本。
/// 旧JSON中的额外字段可自然忽略，无需继续保留废弃的逐行读取属性。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WikiInput {
    pub rel: String,
    #[serde(default)]
    pub content_hash: String,
    #[serde(default)]
    pub source_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WikiOutput {
    pub note_id: String,
    pub path: String,
    pub title: String,
    #[serde(rename = "type")]
    pub page_type: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProcessedInput {
    pub rel: String,
    pub content_hash: String,
    pub disposition: String,
    pub source_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct JobOutputManifest {
    pub version: u32,
    pub outcome: String,
    pub reason_code: Option<String>,
    pub outputs: Vec<WikiOutput>,
    pub processed_inputs: Vec<ProcessedInput>,
    pub remaining_inputs: Vec<WikiInput>,
    pub warnings: Vec<String>,
}

impl JobOutputManifest {
    pub fn generated(
        outputs: Vec<WikiOutput>,
        inputs: &[WikiInput],
        warnings: Vec<String>,
    ) -> Self {
        Self {
            version: 1,
            outcome: "generated".into(),
            reason_code: None,
            outputs,
            processed_inputs: processed(inputs, "used"),
            remaining_inputs: Vec::new(),
            warnings,
        }
    }

    pub fn no_content(inputs: &[WikiInput], reason: &str, warnings: Vec<String>) -> Self {
        Self {
            version: 1,
            outcome: "no_content".into(),
            reason_code: Some(reason.into()),
            outputs: Vec::new(),
            processed_inputs: processed(inputs, "no_content"),
            remaining_inputs: Vec::new(),
            warnings,
        }
    }
}

fn processed(inputs: &[WikiInput], disposition: &str) -> Vec<ProcessedInput> {
    inputs
        .iter()
        .map(|input| ProcessedInput {
            rel: input.rel.clone(),
            content_hash: input.content_hash.clone(),
            disposition: disposition.into(),
            source_ids: input.source_ids.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_input_records_load_without_retaining_read_coverage_fields() {
        let input: WikiInput = serde_json::from_value(serde_json::json!({
            "rel": "work/turns/source.md",
            "content_hash": "processed-version",
            "line_count": 42,
            "source_ids": ["source"]
        }))
        .unwrap();
        assert_eq!(input.source_ids, vec!["source"]);
        assert_eq!(input.content_hash, "processed-version");
        let saved = serde_json::to_value(input).unwrap();
        assert!(saved.get("line_count").is_none());
    }
}
