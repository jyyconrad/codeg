//! Wiki 素材来源表，统一登记 ACP 轮次、本地会话、导入文件和粘贴文本。
//!
//! wiki_service 保存原始地址、提取状态及项目/目录等关联元数据，source_group_id 与
//! previous_source_id 串联同一素材的版本；正文由文件系统保存，read_model 阅读时检查存在性。
//! 原文和归纳页面都可继续编辑，哈希用于去重与来源版本标记，不作为输入冻结约束。

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "wiki_source")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub source_group_id: String,
    pub vault_id: String,
    /// acp-turn / local-session / document / pasted-text
    pub source_kind: String,
    pub source_seq: i64,
    pub run_id: Option<String>,
    pub original_hash: Option<String>,
    pub raw_path: Option<String>,
    pub raw_hash: Option<String>,
    pub extractor_version: Option<String>,
    /// 原文提取结果；沿用数据库列名，对外 DTO 名为 extraction_status。
    pub coverage_status: Option<String>,
    /// processing / awaiting-acceptance / ready / failed / cancelled / withdrawn
    pub eligibility: String,
    pub material_role: Option<String>,
    pub personal_role: Option<String>,
    pub annotation_revision: i32,
    pub conversation_id: Option<i32>,
    pub folder_id: Option<i32>,
    pub root_folder_id: Option<i32>,
    pub agent_type: Option<String>,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub captured_at: Option<DateTimeUtc>,
    pub occurred_at: Option<DateTimeUtc>,
    pub truncated: bool,
    pub redacted: bool,
    pub request_id: Option<String>,
    pub original_filename: Option<String>,
    pub format: Option<String>,
    pub source_title: Option<String>,
    pub source_url: Option<String>,
    pub author: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub project_ids: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub area_ids: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub warnings: Option<String>,
    pub page_count: Option<i32>,
    pub previous_source_id: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
