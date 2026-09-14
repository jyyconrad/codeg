//! 在 Wiki 内容持久化后通知前端刷新目录、正文、资料与任务视图。
//! 桌面和服务器通过 EventEmitter 共用事件；发送失败不回滚已保存的数据。
//! 事件只表示视图需要刷新，不能用来推断模型或文件写入已经成功。

use crate::web::event_bridge::{emit_event, EventEmitter};
use sea_orm::DatabaseConnection;
use std::sync::atomic::{AtomicU64, Ordering};

static REVISION: AtomicU64 = AtomicU64::new(0);

pub async fn content_changed(conn: &DatabaseConnection, emitter: &EventEmitter) {
    let vault_id = crate::db::service::wiki_service::active_vault(conn)
        .await
        .ok()
        .flatten()
        .map(|v| v.id);
    emit_event(
        emitter,
        "wiki://content-changed",
        serde_json::json!({
            "version": 1,
            "revision": REVISION.fetch_add(1, Ordering::Relaxed) + 1,
            "vault_id": vault_id,
            "changed_note_paths": [],
            "changed_source_ids": [],
        }),
    );
}
