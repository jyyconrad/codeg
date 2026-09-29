//! Durable workflow follow-up delivery, one row per
//! `(conversation_id, external_session_id, run_id)`.
//!
//! Claim is compare-and-set (`pending` → `claimed`). Evidence is refreshed only
//! while the row is still `pending`. Logs in this module are ids and status —
//! never `evidence_text`, the prompt, or `failure_reason`.

use chrono::Utc;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveEnum, ActiveModelTrait, ActiveValue::NotSet, ColumnTrait, Condition, DatabaseConnection,
    EntityTrait, QueryFilter, Set,
};

use crate::db::entities::workflow_follow_up::{self, DeliveryStatus};
use crate::db::error::DbError;

/// How many times to retry a compare-and-set that lost a race with another
/// pending writer. A stuck row surfaces as [`DbError::Conflict`] instead of spinning.
const STATE_ATTEMPTS: u8 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowUpKey {
    pub conversation_id: i32,
    pub external_session_id: String,
    pub run_id: String,
}

#[derive(Debug, Clone)]
pub struct FollowUpUpsert {
    pub key: FollowUpKey,
    pub agent_type: String,
    pub terminal_status: String,
    pub workflow_name: String,
    pub result_source: String,
    pub evidence_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpsertPendingOutcome {
    Inserted,
    Updated,
    /// Row exists and is not pending.
    Unchanged,
}

/// Insert a pending delivery, or refresh evidence while the row is still pending.
///
/// A `claimed`, `sent`, `skipped`, or `failed` row is left unchanged and is not
/// moved back to `pending`. Reopening a claimed row is [`reopen_claimed`].
pub async fn upsert_pending(
    conn: &DatabaseConnection,
    upsert: FollowUpUpsert,
) -> Result<UpsertPendingOutcome, DbError> {
    validate_key(&upsert.key)?;
    for _ in 0..STATE_ATTEMPTS {
        match insert_pending(conn, &upsert).await? {
            InsertPending::Inserted => {
                log_delivery(&upsert.key, DeliveryStatus::Pending);
                return Ok(UpsertPendingOutcome::Inserted);
            }
            InsertPending::AlreadyExists => {}
        }
        if refresh_pending(conn, &upsert).await? {
            log_delivery(&upsert.key, DeliveryStatus::Pending);
            return Ok(UpsertPendingOutcome::Updated);
        }
        // Missing: deleted between the unique conflict and this read — insert again.
        // Still pending with different evidence: another writer won the race — retry.
        if let Some(row) = get(conn, &upsert.key).await? {
            match row.delivery_status {
                DeliveryStatus::Pending if same_evidence(&row, &upsert) => {
                    return Ok(UpsertPendingOutcome::Updated);
                }
                DeliveryStatus::Pending => {}
                DeliveryStatus::Claimed
                | DeliveryStatus::Sent
                | DeliveryStatus::Skipped
                | DeliveryStatus::Failed => {
                    return Ok(UpsertPendingOutcome::Unchanged);
                }
            }
        }
    }
    Err(unsettled(&upsert.key, "upsert"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimOutcome {
    Claimed,
    /// Row exists but status is not pending (includes claimed, sent, skipped, failed).
    NotPending,
    Missing,
}

/// Compare-and-set `pending` → `claimed`. A second claim does not succeed.
pub async fn claim(conn: &DatabaseConnection, key: &FollowUpKey) -> Result<ClaimOutcome, DbError> {
    validate_key(key)?;
    for _ in 0..STATE_ATTEMPTS {
        if cas_status(
            conn,
            key,
            &[DeliveryStatus::Pending],
            DeliveryStatus::Claimed,
            None,
        )
        .await?
        {
            log_delivery(key, DeliveryStatus::Claimed);
            return Ok(ClaimOutcome::Claimed);
        }
        let Some(row) = get(conn, key).await? else {
            return Ok(ClaimOutcome::Missing);
        };
        match row.delivery_status {
            // Still pending: the conditional update lost a race. Retry.
            DeliveryStatus::Pending => {}
            DeliveryStatus::Claimed
            | DeliveryStatus::Sent
            | DeliveryStatus::Skipped
            | DeliveryStatus::Failed => return Ok(ClaimOutcome::NotPending),
        }
    }
    Err(unsettled(key, "claim"))
}

/// Record a successful enqueue. Allowed from `pending` or `claimed`.
///
/// Already `sent` is `Ok`. `skipped` and `failed` stay as they are.
/// Missing is [`DbError::NotFound`].
pub async fn mark_sent(conn: &DatabaseConnection, key: &FollowUpKey) -> Result<(), DbError> {
    settle(conn, key, DeliveryStatus::Sent, None).await
}

/// Move `claimed` back to `pending` and replace its evidence.
///
/// `sent`, `skipped`, and `failed` are not reopened. `false` when the row is
/// missing or not `claimed`. Evidence columns change only on the row that
/// moved. `failure_reason` is left as stored and is not logged.
pub async fn reopen_claimed(
    conn: &DatabaseConnection,
    upsert: &FollowUpUpsert,
) -> Result<bool, DbError> {
    validate_key(&upsert.key)?;
    let now = Utc::now();
    let res = workflow_follow_up::Entity::update_many()
        .col_expr(
            workflow_follow_up::Column::DeliveryStatus,
            Expr::value(DeliveryStatus::Pending),
        )
        .col_expr(
            workflow_follow_up::Column::AgentType,
            Expr::value(upsert.agent_type.as_str()),
        )
        .col_expr(
            workflow_follow_up::Column::TerminalStatus,
            Expr::value(upsert.terminal_status.as_str()),
        )
        .col_expr(
            workflow_follow_up::Column::WorkflowName,
            Expr::value(upsert.workflow_name.as_str()),
        )
        .col_expr(
            workflow_follow_up::Column::ResultSource,
            Expr::value(upsert.result_source.as_str()),
        )
        .col_expr(
            workflow_follow_up::Column::EvidenceText,
            Expr::value(upsert.evidence_text.as_str()),
        )
        .col_expr(workflow_follow_up::Column::UpdatedAt, Expr::value(now))
        .filter(key_condition(&upsert.key))
        .filter(workflow_follow_up::Column::DeliveryStatus.eq(DeliveryStatus::Claimed))
        .exec(conn)
        .await?;
    if res.rows_affected > 0 {
        log_delivery(&upsert.key, DeliveryStatus::Pending);
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Move `claimed` back to `pending`.
///
/// Already `pending` is a no-op. Missing is `Ok`. `sent`, `skipped`, and
/// `failed` stay as they are and return `Ok` (not an error, and not a rewind).
pub async fn revert_claim_to_pending(
    conn: &DatabaseConnection,
    key: &FollowUpKey,
) -> Result<(), DbError> {
    validate_key(key)?;
    if cas_status(
        conn,
        key,
        &[DeliveryStatus::Claimed],
        DeliveryStatus::Pending,
        None,
    )
    .await?
    {
        log_delivery(key, DeliveryStatus::Pending);
    }
    Ok(())
}

/// Set `failed` and store `reason`, from `pending` or `claimed`.
///
/// Does not overwrite `sent`, `skipped`, or an existing `failed` row.
/// Missing is [`DbError::NotFound`]. `reason` is stored as given and is not logged.
pub async fn mark_failed(
    conn: &DatabaseConnection,
    key: &FollowUpKey,
    reason: &str,
) -> Result<(), DbError> {
    settle(conn, key, DeliveryStatus::Failed, Some(reason)).await
}

/// Set `skipped` and store `reason`, from `pending` or `claimed`.
///
/// Does not overwrite `sent`, `failed`, or an existing `skipped` row.
/// Missing is [`DbError::NotFound`]. `reason` is stored as given and is not logged.
pub async fn mark_skipped(
    conn: &DatabaseConnection,
    key: &FollowUpKey,
    reason: &str,
) -> Result<(), DbError> {
    settle(conn, key, DeliveryStatus::Skipped, Some(reason)).await
}

pub async fn get(
    conn: &DatabaseConnection,
    key: &FollowUpKey,
) -> Result<Option<workflow_follow_up::Model>, DbError> {
    validate_key(key)?;
    Ok(workflow_follow_up::Entity::find()
        .filter(key_condition(key))
        .one(conn)
        .await?)
}

fn validate_key(key: &FollowUpKey) -> Result<(), DbError> {
    if key.external_session_id.trim().is_empty() {
        return Err(DbError::Validation(
            "external_session_id must not be empty".to_string(),
        ));
    }
    if key.run_id.trim().is_empty() {
        return Err(DbError::Validation("run_id must not be empty".to_string()));
    }
    Ok(())
}

enum InsertPending {
    Inserted,
    AlreadyExists,
}

async fn insert_pending(
    conn: &DatabaseConnection,
    upsert: &FollowUpUpsert,
) -> Result<InsertPending, DbError> {
    let now = Utc::now();
    let active = workflow_follow_up::ActiveModel {
        id: NotSet,
        conversation_id: Set(upsert.key.conversation_id),
        external_session_id: Set(upsert.key.external_session_id.clone()),
        run_id: Set(upsert.key.run_id.clone()),
        agent_type: Set(upsert.agent_type.clone()),
        delivery_status: Set(DeliveryStatus::Pending),
        terminal_status: Set(upsert.terminal_status.clone()),
        workflow_name: Set(upsert.workflow_name.clone()),
        result_source: Set(upsert.result_source.clone()),
        evidence_text: Set(upsert.evidence_text.clone()),
        failure_reason: NotSet,
        created_at: Set(now),
        updated_at: Set(now),
    };
    match active.insert(conn).await {
        Ok(_) => Ok(InsertPending::Inserted),
        Err(err) if unique_violation(&err) => Ok(InsertPending::AlreadyExists),
        Err(err) => Err(err.into()),
    }
}

/// Refresh evidence only while `delivery_status` is still `pending`.
async fn refresh_pending(
    conn: &DatabaseConnection,
    upsert: &FollowUpUpsert,
) -> Result<bool, DbError> {
    let now = Utc::now();
    let res = workflow_follow_up::Entity::update_many()
        .col_expr(
            workflow_follow_up::Column::AgentType,
            Expr::value(upsert.agent_type.as_str()),
        )
        .col_expr(
            workflow_follow_up::Column::TerminalStatus,
            Expr::value(upsert.terminal_status.as_str()),
        )
        .col_expr(
            workflow_follow_up::Column::WorkflowName,
            Expr::value(upsert.workflow_name.as_str()),
        )
        .col_expr(
            workflow_follow_up::Column::ResultSource,
            Expr::value(upsert.result_source.as_str()),
        )
        .col_expr(
            workflow_follow_up::Column::EvidenceText,
            Expr::value(upsert.evidence_text.as_str()),
        )
        .col_expr(workflow_follow_up::Column::UpdatedAt, Expr::value(now))
        .filter(key_condition(&upsert.key))
        .filter(workflow_follow_up::Column::DeliveryStatus.eq(DeliveryStatus::Pending))
        .exec(conn)
        .await?;
    Ok(res.rows_affected > 0)
}

fn same_evidence(row: &workflow_follow_up::Model, upsert: &FollowUpUpsert) -> bool {
    row.agent_type == upsert.agent_type
        && row.terminal_status == upsert.terminal_status
        && row.workflow_name == upsert.workflow_name
        && row.result_source == upsert.result_source
        && row.evidence_text == upsert.evidence_text
}

async fn settle(
    conn: &DatabaseConnection,
    key: &FollowUpKey,
    to: DeliveryStatus,
    reason: Option<&str>,
) -> Result<(), DbError> {
    validate_key(key)?;
    for _ in 0..STATE_ATTEMPTS {
        if cas_status(
            conn,
            key,
            &[DeliveryStatus::Pending, DeliveryStatus::Claimed],
            to,
            reason,
        )
        .await?
        {
            log_delivery(key, to);
            return Ok(());
        }
        let Some(row) = get(conn, key).await? else {
            return Err(not_found(key));
        };
        match row.delivery_status {
            // Still open: the conditional update lost a race. Retry.
            DeliveryStatus::Pending | DeliveryStatus::Claimed => {}
            // Terminal rows are not rewound. `sent` is never overwritten.
            DeliveryStatus::Sent | DeliveryStatus::Skipped | DeliveryStatus::Failed => {
                return Ok(());
            }
        }
    }
    Err(unsettled(key, "status update"))
}

/// Conditional delivery-status write. Evidence columns are never included.
/// `true` when a row currently in `from` was updated.
async fn cas_status(
    conn: &DatabaseConnection,
    key: &FollowUpKey,
    from: &[DeliveryStatus],
    to: DeliveryStatus,
    reason: Option<&str>,
) -> Result<bool, DbError> {
    debug_assert!(!from.is_empty());
    let now = Utc::now();
    let mut update = workflow_follow_up::Entity::update_many()
        .col_expr(workflow_follow_up::Column::DeliveryStatus, Expr::value(to))
        .col_expr(workflow_follow_up::Column::UpdatedAt, Expr::value(now));
    if let Some(reason) = reason {
        update = update.col_expr(
            workflow_follow_up::Column::FailureReason,
            Expr::value(reason),
        );
    }
    let res = update
        .filter(key_condition(key))
        .filter(status_in(from))
        .exec(conn)
        .await?;
    Ok(res.rows_affected > 0)
}

fn key_condition(key: &FollowUpKey) -> Condition {
    Condition::all()
        .add(workflow_follow_up::Column::ConversationId.eq(key.conversation_id))
        .add(workflow_follow_up::Column::ExternalSessionId.eq(key.external_session_id.as_str()))
        .add(workflow_follow_up::Column::RunId.eq(key.run_id.as_str()))
}

fn status_in(statuses: &[DeliveryStatus]) -> Condition {
    let mut cond = Condition::any();
    for status in statuses {
        cond = cond.add(workflow_follow_up::Column::DeliveryStatus.eq(*status));
    }
    cond
}

fn unique_violation(err: &sea_orm::DbErr) -> bool {
    err.to_string().contains("UNIQUE constraint failed")
}

fn not_found(key: &FollowUpKey) -> DbError {
    let conversation_id = key.conversation_id;
    let external_session_id = &key.external_session_id;
    let run_id = &key.run_id;
    DbError::NotFound(format!(
        "workflow follow-up not found conversation_id={conversation_id} \
         external_session_id={external_session_id} run_id={run_id}"
    ))
}

fn unsettled(key: &FollowUpKey, action: &str) -> DbError {
    let conversation_id = key.conversation_id;
    let external_session_id = &key.external_session_id;
    let run_id = &key.run_id;
    DbError::Conflict(format!(
        "workflow follow-up {action} did not settle for conversation_id={conversation_id} \
         external_session_id={external_session_id} run_id={run_id}"
    ))
}

/// Ids and delivery status only. `evidence_text`, the prompt, and `failure_reason`
/// are intentionally not fields here.
fn log_delivery(key: &FollowUpKey, status: DeliveryStatus) {
    let status = status.to_value();
    tracing::info!(
        conversation_id = key.conversation_id,
        external_session_id = %key.external_session_id,
        run_id = %key.run_id,
        status = %status,
        "workflow follow-up delivery"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_helpers::{fresh_in_memory_db, seed_conversation, seed_folder};
    use crate::models::AgentType;

    fn sample(conversation_id: i32, run_id: &str) -> FollowUpUpsert {
        FollowUpUpsert {
            key: FollowUpKey {
                conversation_id,
                external_session_id: "ext-session".to_string(),
                run_id: run_id.to_string(),
            },
            agent_type: "grok".to_string(),
            terminal_status: "completed".to_string(),
            workflow_name: "release".to_string(),
            result_source: "final_summary".to_string(),
            evidence_text: "first conclusion".to_string(),
        }
    }

    async fn seed() -> (crate::db::AppDatabase, i32) {
        let db = fresh_in_memory_db().await;
        let folder_id = seed_folder(&db, "/tmp/workflow-follow-up").await;
        let conversation_id = seed_conversation(&db, folder_id, AgentType::Grok).await;
        (db, conversation_id)
    }

    async fn row(db: &crate::db::AppDatabase, key: &FollowUpKey) -> workflow_follow_up::Model {
        get(&db.conn, key).await.unwrap().expect("follow-up row")
    }

    fn assert_evidence(got: &workflow_follow_up::Model, want: &FollowUpUpsert) {
        assert_eq!(got.agent_type, want.agent_type);
        assert_eq!(got.terminal_status, want.terminal_status);
        assert_eq!(got.workflow_name, want.workflow_name);
        assert_eq!(got.result_source, want.result_source);
        assert_eq!(got.evidence_text, want.evidence_text);
    }

    #[tokio::test]
    async fn upsert_inserts_pending() {
        let (db, conversation_id) = seed().await;
        let upsert = sample(conversation_id, "run-1");

        assert_eq!(
            upsert_pending(&db.conn, upsert.clone()).await.unwrap(),
            UpsertPendingOutcome::Inserted
        );

        let stored = row(&db, &upsert.key).await;
        assert_eq!(stored.delivery_status, DeliveryStatus::Pending);
        assert_eq!(stored.conversation_id, conversation_id);
        assert_eq!(stored.external_session_id, "ext-session");
        assert_eq!(stored.run_id, "run-1");
        assert_eq!(stored.failure_reason, None);
        assert_evidence(&stored, &upsert);
    }

    #[tokio::test]
    async fn second_upsert_while_pending_updates_evidence_and_terminal_status() {
        let (db, conversation_id) = seed().await;
        let original = sample(conversation_id, "run-1");
        upsert_pending(&db.conn, original.clone()).await.unwrap();
        let inserted = row(&db, &original.key).await;

        let mut revised = original.clone();
        revised.agent_type = "claude".to_string();
        revised.terminal_status = "failed".to_string();
        revised.workflow_name = "release-v2".to_string();
        revised.result_source = "error_detail".to_string();
        revised.evidence_text = "revised conclusion".to_string();

        assert_eq!(
            upsert_pending(&db.conn, revised.clone()).await.unwrap(),
            UpsertPendingOutcome::Updated
        );

        let stored = row(&db, &original.key).await;
        assert_eq!(stored.id, inserted.id);
        assert_eq!(stored.delivery_status, DeliveryStatus::Pending);
        assert_evidence(&stored, &revised);
    }

    #[tokio::test]
    async fn claim_once_then_second_claim_returns_not_pending() {
        let (db, conversation_id) = seed().await;
        let upsert = sample(conversation_id, "run-1");
        upsert_pending(&db.conn, upsert.clone()).await.unwrap();

        assert_eq!(
            claim(&db.conn, &upsert.key).await.unwrap(),
            ClaimOutcome::Claimed
        );
        assert_eq!(
            claim(&db.conn, &upsert.key).await.unwrap(),
            ClaimOutcome::NotPending
        );

        let stored = row(&db, &upsert.key).await;
        assert_eq!(stored.delivery_status, DeliveryStatus::Claimed);
        assert_evidence(&stored, &upsert);
    }

    #[tokio::test]
    async fn upsert_after_claim_does_not_change_evidence() {
        let (db, conversation_id) = seed().await;
        let original = sample(conversation_id, "run-1");
        upsert_pending(&db.conn, original.clone()).await.unwrap();
        claim(&db.conn, &original.key).await.unwrap();

        let mut revised = original.clone();
        revised.agent_type = "claude".to_string();
        revised.terminal_status = "failed".to_string();
        revised.workflow_name = "other".to_string();
        revised.result_source = "unavailable".to_string();
        revised.evidence_text = "should not land".to_string();

        assert_eq!(
            upsert_pending(&db.conn, revised).await.unwrap(),
            UpsertPendingOutcome::Unchanged
        );
        let stored = row(&db, &original.key).await;
        assert_eq!(stored.delivery_status, DeliveryStatus::Claimed);
        assert_evidence(&stored, &original);
    }

    #[tokio::test]
    async fn upsert_after_mark_sent_leaves_evidence_unchanged() {
        let (db, conversation_id) = seed().await;
        let original = sample(conversation_id, "run-1");
        upsert_pending(&db.conn, original.clone()).await.unwrap();
        claim(&db.conn, &original.key).await.unwrap();
        mark_sent(&db.conn, &original.key).await.unwrap();

        let mut revised = original.clone();
        revised.terminal_status = "failed".to_string();
        revised.evidence_text = "late revision".to_string();

        assert_eq!(
            upsert_pending(&db.conn, revised).await.unwrap(),
            UpsertPendingOutcome::Unchanged
        );
        let stored = row(&db, &original.key).await;
        assert_eq!(stored.delivery_status, DeliveryStatus::Sent);
        assert_evidence(&stored, &original);
    }

    #[tokio::test]
    async fn revert_claim_to_pending_on_claimed_allows_claim_again() {
        let (db, conversation_id) = seed().await;
        let upsert = sample(conversation_id, "run-1");
        upsert_pending(&db.conn, upsert.clone()).await.unwrap();

        revert_claim_to_pending(&db.conn, &upsert.key)
            .await
            .unwrap();
        assert_eq!(
            row(&db, &upsert.key).await.delivery_status,
            DeliveryStatus::Pending
        );

        assert_eq!(
            claim(&db.conn, &upsert.key).await.unwrap(),
            ClaimOutcome::Claimed
        );
        revert_claim_to_pending(&db.conn, &upsert.key)
            .await
            .unwrap();
        assert_eq!(
            row(&db, &upsert.key).await.delivery_status,
            DeliveryStatus::Pending
        );
        assert_eq!(
            claim(&db.conn, &upsert.key).await.unwrap(),
            ClaimOutcome::Claimed
        );
        assert_eq!(
            claim(&db.conn, &upsert.key).await.unwrap(),
            ClaimOutcome::NotPending
        );
    }

    #[tokio::test]
    async fn revert_claim_to_pending_on_sent_does_not_return_to_pending() {
        let (db, conversation_id) = seed().await;
        let upsert = sample(conversation_id, "run-1");
        upsert_pending(&db.conn, upsert.clone()).await.unwrap();
        claim(&db.conn, &upsert.key).await.unwrap();
        mark_sent(&db.conn, &upsert.key).await.unwrap();

        revert_claim_to_pending(&db.conn, &upsert.key)
            .await
            .unwrap();

        let stored = row(&db, &upsert.key).await;
        assert_eq!(stored.delivery_status, DeliveryStatus::Sent);
        assert_evidence(&stored, &upsert);
        assert_eq!(
            claim(&db.conn, &upsert.key).await.unwrap(),
            ClaimOutcome::NotPending
        );
    }

    #[tokio::test]
    async fn mark_failed_and_mark_skipped_persist_reason() {
        let (db, conversation_id) = seed().await;

        let failed = sample(conversation_id, "run-failed");
        upsert_pending(&db.conn, failed.clone()).await.unwrap();
        claim(&db.conn, &failed.key).await.unwrap();
        mark_failed(&db.conn, &failed.key, "connection closed")
            .await
            .unwrap();
        let failed_row = row(&db, &failed.key).await;
        assert_eq!(failed_row.delivery_status, DeliveryStatus::Failed);
        assert_eq!(
            failed_row.failure_reason.as_deref(),
            Some("connection closed")
        );
        assert_evidence(&failed_row, &failed);
        revert_claim_to_pending(&db.conn, &failed.key)
            .await
            .unwrap();
        assert_eq!(
            row(&db, &failed.key).await.delivery_status,
            DeliveryStatus::Failed
        );

        let mut late = failed.clone();
        late.evidence_text = "should not replace".to_string();
        assert_eq!(
            upsert_pending(&db.conn, late).await.unwrap(),
            UpsertPendingOutcome::Unchanged
        );
        assert_eq!(
            row(&db, &failed.key).await.evidence_text,
            failed.evidence_text
        );

        let skipped = sample(conversation_id, "run-skipped");
        upsert_pending(&db.conn, skipped.clone()).await.unwrap();
        mark_skipped(&db.conn, &skipped.key, "non-grok agent")
            .await
            .unwrap();
        let skipped_row = row(&db, &skipped.key).await;
        assert_eq!(skipped_row.delivery_status, DeliveryStatus::Skipped);
        assert_eq!(
            skipped_row.failure_reason.as_deref(),
            Some("non-grok agent")
        );
        assert_evidence(&skipped_row, &skipped);
        revert_claim_to_pending(&db.conn, &skipped.key)
            .await
            .unwrap();
        assert_eq!(
            row(&db, &skipped.key).await.delivery_status,
            DeliveryStatus::Skipped
        );
    }

    #[tokio::test]
    async fn different_run_id_is_a_separate_row() {
        let (db, conversation_id) = seed().await;
        let first = sample(conversation_id, "run-a");
        let mut second = sample(conversation_id, "run-b");
        second.evidence_text = "other run".to_string();

        assert_eq!(
            upsert_pending(&db.conn, first.clone()).await.unwrap(),
            UpsertPendingOutcome::Inserted
        );
        assert_eq!(
            upsert_pending(&db.conn, second.clone()).await.unwrap(),
            UpsertPendingOutcome::Inserted
        );

        assert_eq!(
            claim(&db.conn, &first.key).await.unwrap(),
            ClaimOutcome::Claimed
        );

        let first_row = row(&db, &first.key).await;
        let second_row = row(&db, &second.key).await;
        assert_ne!(first_row.id, second_row.id);
        assert_eq!(first_row.delivery_status, DeliveryStatus::Claimed);
        assert_eq!(second_row.delivery_status, DeliveryStatus::Pending);
        assert_evidence(&first_row, &first);
        assert_evidence(&second_row, &second);
    }

    #[tokio::test]
    async fn claim_of_unknown_key_returns_missing() {
        let (db, conversation_id) = seed().await;
        let key = sample(conversation_id, "missing").key;

        assert_eq!(claim(&db.conn, &key).await.unwrap(), ClaimOutcome::Missing);
        revert_claim_to_pending(&db.conn, &key).await.unwrap();
        assert!(get(&db.conn, &key).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn mark_sent_is_not_replaced_by_later_mark_failed() {
        let (db, conversation_id) = seed().await;
        let upsert = sample(conversation_id, "run-1");
        upsert_pending(&db.conn, upsert.clone()).await.unwrap();
        mark_sent(&db.conn, &upsert.key).await.unwrap();
        mark_failed(&db.conn, &upsert.key, "connection closed")
            .await
            .unwrap();
        mark_skipped(&db.conn, &upsert.key, "non-grok agent")
            .await
            .unwrap();

        let stored = row(&db, &upsert.key).await;
        assert_eq!(stored.delivery_status, DeliveryStatus::Sent);
        assert_eq!(stored.failure_reason, None);
        assert_evidence(&stored, &upsert);
    }

    #[tokio::test]
    async fn empty_external_session_id_or_run_id_is_rejected() {
        let (db, conversation_id) = seed().await;

        let mut bad_session = sample(conversation_id, "run-1");
        bad_session.key.external_session_id = "  ".to_string();
        let err = upsert_pending(&db.conn, bad_session).await.unwrap_err();
        let message = err.to_string();
        assert!(message.contains("external_session_id"), "{message}");

        let mut bad_run = sample(conversation_id, "run-1");
        bad_run.key.run_id.clear();
        let err = upsert_pending(&db.conn, bad_run).await.unwrap_err();
        let message = err.to_string();
        assert!(message.contains("run_id"), "{message}");

        let err = claim(
            &db.conn,
            &FollowUpKey {
                conversation_id,
                external_session_id: String::new(),
                run_id: "run-1".to_string(),
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DbError::Validation(_)), "{err}");
    }

    #[tokio::test]
    async fn mark_sent_of_unknown_key_is_not_found() {
        let (db, conversation_id) = seed().await;
        let key = sample(conversation_id, "missing").key;
        let err = mark_sent(&db.conn, &key).await.unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)), "{err}");
    }

    #[tokio::test]
    async fn reopen_claimed_refreshes_evidence_only_while_claimed() {
        let (db, conversation_id) = seed().await;
        let original = sample(conversation_id, "run-1");
        upsert_pending(&db.conn, original.clone()).await.unwrap();
        claim(&db.conn, &original.key).await.unwrap();

        let mut revised = original.clone();
        revised.terminal_status = "failed".to_string();
        revised.result_source = "error_detail".to_string();
        revised.evidence_text = "disk full".to_string();
        assert!(reopen_claimed(&db.conn, &revised).await.unwrap());
        let reopened = row(&db, &original.key).await;
        assert_eq!(reopened.delivery_status, DeliveryStatus::Pending);
        assert_evidence(&reopened, &revised);

        claim(&db.conn, &original.key).await.unwrap();
        mark_sent(&db.conn, &original.key).await.unwrap();
        revised.evidence_text = "should not replace sent".to_string();
        assert!(!reopen_claimed(&db.conn, &revised).await.unwrap());
        let sent = row(&db, &original.key).await;
        assert_eq!(sent.delivery_status, DeliveryStatus::Sent);
        assert_eq!(sent.evidence_text, "disk full");

        let skipped = sample(conversation_id, "run-skipped");
        upsert_pending(&db.conn, skipped.clone()).await.unwrap();
        mark_skipped(&db.conn, &skipped.key, "host wake disabled")
            .await
            .unwrap();
        assert!(!reopen_claimed(&db.conn, &skipped).await.unwrap());
        assert_eq!(
            row(&db, &skipped.key).await.delivery_status,
            DeliveryStatus::Skipped
        );

        let failed = sample(conversation_id, "run-failed");
        upsert_pending(&db.conn, failed.clone()).await.unwrap();
        mark_failed(&db.conn, &failed.key, "connection closed")
            .await
            .unwrap();
        assert!(!reopen_claimed(&db.conn, &failed).await.unwrap());
        assert_eq!(
            row(&db, &failed.key).await.delivery_status,
            DeliveryStatus::Failed
        );

        assert!(
            !reopen_claimed(&db.conn, &sample(conversation_id, "missing"))
                .await
                .unwrap()
        );
    }
}
