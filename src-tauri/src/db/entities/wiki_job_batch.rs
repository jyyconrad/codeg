//! Wiki 归纳批次的完成记录，用于文件提交后的数据库恢复与去重。
//!
//! wiki_pipeline_service 在同一事务内写入批次、来源关联、已消费记录和任务结果；
//! (job_id, attempt, batch_id) 与 manifest_hash 让重启后重放同一提交保持幂等。
//! 本表对应已生成页面的提交记录，不保存或冻结输入素材正文。

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "wiki_job_batch")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub job_id: String,
    pub attempt: i32,
    pub batch_id: String,
    pub manifest_path: String,
    pub manifest_hash: String,
    #[sea_orm(column_type = "Text")]
    pub result_json: String,
    pub finalized_at: DateTimeUtc,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}
