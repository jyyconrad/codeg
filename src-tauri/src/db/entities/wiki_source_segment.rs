use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "wiki_source_segment")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub source_id: String,
    pub segment_id: String,
    pub content_hash: String,
    #[sea_orm(column_type = "Text")]
    pub locator: Option<String>,
    pub annotation_revision: i32,
    pub compile_contract_version: Option<String>,
    pub analysis_config_version: Option<String>,
    pub stage: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub artifact: Option<String>,
    pub status: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
