use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "wiki_job")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub vault_id: String,
    pub source_id: Option<String>,
    /// ingest / compile
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
    pub started_at: Option<DateTimeUtc>,
    pub finished_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
