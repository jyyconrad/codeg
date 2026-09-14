//! Wiki 资料库注册表，以规范化物理目录关联 Markdown 文件与来源、项目及后台任务。
//!
//! wiki_service 负责切换当前资料库，engine 使用 next_compile_at 安排下一次知识归纳；
//! 用户配置由 wiki/settings 管理，目录布局及 Obsidian 可读页面由 wiki/vault 管理。

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "wiki_vault")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub canonical_path: String,
    pub config_revision: i32,
    pub next_compile_at: Option<DateTimeUtc>,
    pub is_active: bool,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
