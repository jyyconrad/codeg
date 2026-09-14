//! 为现有 Wiki 任务增加重试时间和逻辑任务唯一约束。
//! 保留已执行数据库使用的历史迁移 ID；不清空 Wiki 数据、配置或删除旧表，
//! 正文目录的直接复制与路径调整由用户迁入操作完成，不在结构迁移中执行。

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("ALTER TABLE wiki_job ADD COLUMN next_attempt_at TEXT NULL")
            .await?;
        // 相同逻辑任务通过更新原行重试，唯一约束负责并发去重；不以删除旧任务换取建索引。
        db.execute_unprepared(
            "CREATE UNIQUE INDEX idx_wiki_job_logical_key ON wiki_job(vault_id, dedupe_key) WHERE dedupe_key IS NOT NULL",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Migration(
            "This historical Wiki migration is not rolled back automatically".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database};

    #[tokio::test]
    async fn retry_schema_preserves_existing_wiki_rows_settings_and_legacy_tables() {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let manager = SchemaManager::new(&db);
        for table in [
            "wiki_contribution",
            "wiki_compile_input",
            "wiki_source_segment",
            "wiki_source",
            "wiki_project_binding",
            "wiki_vault",
            "conversation",
        ] {
            db.execute_unprepared(&format!("CREATE TABLE {table}(id TEXT PRIMARY KEY)"))
                .await
                .unwrap();
            db.execute_unprepared(&format!("INSERT INTO {table} VALUES ('existing')"))
                .await
                .unwrap();
        }
        db.execute_unprepared("CREATE TABLE wiki_job(id TEXT, vault_id TEXT, dedupe_key TEXT)")
            .await
            .unwrap();
        db.execute_unprepared("INSERT INTO wiki_job VALUES ('job', 'vault', 'key')")
            .await
            .unwrap();
        db.execute_unprepared("CREATE TABLE app_metadata(key TEXT, value TEXT)")
            .await
            .unwrap();
        db.execute_unprepared("INSERT INTO app_metadata VALUES ('wiki_settings','old'),('wiki_db_instance_id','old'),('chat_settings','keep')").await.unwrap();
        Migration.up(&manager).await.unwrap();
        assert_eq!(Migration.name(), "m20260914_000001_wiki_v2_reset");
        for table in [
            "wiki_job",
            "wiki_source",
            "wiki_vault",
            "wiki_contribution",
            "wiki_compile_input",
            "wiki_source_segment",
            "wiki_project_binding",
        ] {
            let count = db
                .query_one(sea_orm::Statement::from_string(
                    sea_orm::DatabaseBackend::Sqlite,
                    format!("SELECT count(*) AS n FROM {table}"),
                ))
                .await
                .unwrap()
                .unwrap()
                .try_get::<i64>("", "n")
                .unwrap();
            assert_eq!(count, 1, "{table} must retain existing data");
        }
        assert!(manager.has_table("wiki_compile_input").await.unwrap());
        assert!(manager.has_table("wiki_source_segment").await.unwrap());
        assert!(manager
            .has_column("wiki_job", "next_attempt_at")
            .await
            .unwrap());
        let row = db.query_one(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT (SELECT count(*) FROM conversation) AS conversations, (SELECT value FROM app_metadata WHERE key='chat_settings') AS setting, (SELECT value FROM app_metadata WHERE key='wiki_settings') AS wiki_setting, (SELECT value FROM app_metadata WHERE key='wiki_db_instance_id') AS instance_id".to_string(),
        )).await.unwrap().unwrap();
        assert_eq!(row.try_get::<i64>("", "conversations").unwrap(), 1);
        assert_eq!(row.try_get::<String>("", "setting").unwrap(), "keep");
        assert_eq!(row.try_get::<String>("", "wiki_setting").unwrap(), "old");
        assert_eq!(row.try_get::<String>("", "instance_id").unwrap(), "old");
        assert!(db
            .execute_unprepared(
                "INSERT INTO wiki_job(id,vault_id,dedupe_key) VALUES ('duplicate','vault','key')"
            )
            .await
            .is_err());
        db.execute_unprepared(
            "INSERT INTO wiki_job(id,vault_id,dedupe_key) VALUES ('other-vault','other','key')",
        )
        .await
        .unwrap();

        assert!(Migration.down(&manager).await.is_err());
    }
}
