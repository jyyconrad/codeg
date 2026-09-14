//! Wiki 项目身份与 Codeg 根目录的绑定表，支持来源、项目页面及仓库元数据的关联。
//!
//! wiki_service 按资料库、数据库实例、root_folder_id 创建稳定身份；
//! wiki/project_metadata 以此补充当前目录和 Git 仓库信息，目录名称/路径仍从 folder 读取。

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "wiki_project_binding")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub vault_id: String,
    pub db_instance_id: String,
    pub root_folder_id: i32,
    pub project_note_id: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
