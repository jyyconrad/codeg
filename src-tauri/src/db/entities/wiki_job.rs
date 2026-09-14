//! Wiki 后台任务表，保存轮次记录、会话汇总、知识归纳的当前调度状态。
//!
//! engine 经 wiki_service 创建、认领和重试任务；dedupe_key 标识逻辑任务，
//! attempt 区分执行次数，历史详情存入 wiki_job_attempt，分批提交存入 wiki_job_batch。
//! input/output_manifest 是任务参数与产出记录，Wiki 正文仍保存在资料库 Markdown 中。

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "wiki_job")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub vault_id: String,
    pub source_id: Option<String>,
    /// turn_summary / session_rollup / wiki_synthesize
    pub kind: String,
    /// queued / running / succeeded / failed / cancelled
    pub status: String,
    pub dedupe_key: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub input_manifest: Option<String>,
    pub config_version: Option<String>,
    pub model_id: Option<String>,
    pub protocol: Option<String>,
    pub attempt: i32,
    pub error_code: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub error_message: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub output_manifest: Option<String>,
    pub next_attempt_at: Option<DateTimeUtc>,
    pub started_at: Option<DateTimeUtc>,
    pub finished_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
