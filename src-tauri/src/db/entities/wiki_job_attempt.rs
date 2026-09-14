//! Wiki 任务每次执行的详情表，为处理记录页保留重试前的错误、模型及产出信息。
//!
//! wiki_service 在认领、更新结果和结束任务时按 (job_id, attempt) 写入；
//! wiki_job 保留当前状态，本表保留既往尝试，避免重试覆盖原来的失败原因。

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "wiki_job_attempt")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub job_id: String,
    pub attempt: i32,
    pub status: String,
    #[sea_orm(column_type = "Text")]
    pub input_manifest: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub output_manifest: Option<String>,
    pub model_id: Option<String>,
    pub protocol: Option<String>,
    pub error_code: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub error_message: Option<String>,
    pub started_at: Option<DateTimeUtc>,
    pub finished_at: Option<DateTimeUtc>,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}
