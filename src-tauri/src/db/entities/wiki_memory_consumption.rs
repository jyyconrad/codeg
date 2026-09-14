//! 记录工作笔记已经参与哪次 Wiki 归纳，供后续调度跳过未变化的已处理内容。
//!
//! wiki_pipeline_service 随批次提交写入，compile 按资料库、笔记路径、内容哈希和
//! 生成契约版本查询；笔记内容变化后可以再次归纳，原 Markdown 文件可继续编辑。

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "wiki_memory_consumption")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub vault_id: String,
    pub input_kind: String,
    pub input_ref: String,
    pub content_hash: String,
    pub contract_version: String,
    pub job_id: String,
    pub batch_id: String,
    pub disposition: String,
    pub created_at: DateTimeUtc,
}
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}
