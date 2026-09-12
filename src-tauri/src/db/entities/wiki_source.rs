use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "wiki_source")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub source_group_id: String,
    pub vault_id: String,
    /// acp-turn / document / pasted-text
    pub source_kind: String,
    pub source_seq: i64,
    pub run_id: Option<String>,
    pub original_hash: Option<String>,
    pub raw_path: Option<String>,
    pub raw_hash: Option<String>,
    pub extractor_version: Option<String>,
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
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
