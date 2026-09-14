//! Wiki 页面与资料来源的关联表，用于笔记详情反查来源及资料详情查看产出。
//!
//! 轮次记录由 wiki_service 写入，归纳结果由 wiki_pipeline_service 随批次原子写入；
//! raw_hash/annotation_revision 记录关联建立时的版本，不限制后续编辑原文。
//! claim/evidence/case 列保留已有表结构映射，当前业务只建立页面级来源关系。

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "wiki_contribution")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub source_id: String,
    pub raw_hash: String,
    pub annotation_revision: i32,
    pub note_id: String,
    pub claim_id: Option<String>,
    pub evidence_id: Option<String>,
    pub case_id: Option<String>,
    pub commit_id: Option<String>,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
