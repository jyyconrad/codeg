//! 综合整理行为测试：来源关联、去重、批次重试与输出恢复。

use super::*;
use tempfile::tempdir;

fn input() -> WikiInput {
    WikiInput {
        rel: "work/turns/source.md".into(),
        content_hash: "input-hash".into(),

        source_ids: vec!["source".into()],
    }
}
fn valid_output() -> Value {
    json!({"schema":SYNTHESIZE_CONTRACT_VERSION, "page_proposals":[{"proposal_key":"cursor", "op":"create", "type":"method", "title":"游标分页", "summary":"检查连续翻页的边界行为。", "body":"## 适用场景\n核对空页、重复数据与稳定排序。", "input_rels":["work/turns/source.md"]}], "warnings":[]})
}

#[test]
fn proposal_gets_host_identity_path_and_clickable_sources() {
    let result = validate_output(valid_output(), &[input()], &[]).unwrap();
    assert_eq!(result.proposals.len(), 1);
    let proposal = &result.proposals[0];
    assert!(proposal.rel.starts_with("knowledge/methods/"));
    assert!(Uuid::parse_str(&yaml_string(&proposal.after, "codeg_note_id").unwrap()).is_ok());
    assert!(proposal.after.contains("work/turns/source.md"));
    assert_eq!(result.processed[0].source_ids, vec!["source"]);
}
#[test]
fn agent_can_return_notes_without_hashes_line_evidence_or_read_receipts() {
    let out = json!({"schema":SYNTHESIZE_CONTRACT_VERSION,"page_proposals":[{
        "proposal_key":"method", "op":"create", "type":"method", "title":"阅读整理",
        "body":"自主整理材料，并链接已有 [[knowledge/methods/example]]。"
    }]});
    let result = validate_output(out, &[input()], &[]).unwrap();
    assert_eq!(result.processed[0].disposition, "used");
    assert!(result.proposals[0].after.contains("source_ids:"));
    assert!(result.proposals[0].after.contains("work/turns/source.md"));
}
#[test]
fn an_empty_proposal_list_needs_no_input_hash_attestation() {
    let result = validate_output(
        json!({"schema":SYNTHESIZE_CONTRACT_VERSION,"page_proposals":[]}),
        &[input()],
        &[],
    )
    .unwrap();
    assert!(result.proposals.is_empty());
    assert_eq!(result.processed[0].disposition, "no_content");
}
#[test]
fn updated_input_is_read_without_a_frozen_version_check() {
    let dir = tempdir().unwrap();
    vault::initialize_vault(dir.path()).unwrap();
    fs::write(
        dir.path().join("work/turns/source.md"),
        "Updated source material.",
    )
    .unwrap();
    let reference = MemoryNoteRef {
        rel: input().rel,
        content_hash: "earlier-version".into(),
        ..Default::default()
    };
    let loaded = load_memory(dir.path(), &reference).unwrap();
    assert_eq!(loaded.text, "Updated source material.");
    assert_eq!(loaded.input.content_hash, content_hash(&loaded.text));
}
#[test]
fn batches_respect_count_chars_and_context_without_losing_inputs() {
    let notes = (0..19)
        .map(|i| LoadedMemory {
            reference: MemoryNoteRef {
                rel: format!("work/turns/{i:02}.md"),
                content_hash: format!("hash{i}"),
                ..Default::default()
            },
            text: "a".repeat(9000),
            input: WikiInput {
                rel: format!("work/turns/{i:02}.md"),
                content_hash: format!("hash{i}"),

                source_ids: vec![],
            },
        })
        .collect::<Vec<_>>();
    let batches = plan_batches(notes, 100_000);
    assert_eq!(batches.iter().map(Vec::len).sum::<usize>(), 19);
    assert!(batches.iter().all(|batch| batch.len() <= 6));
    let tight = plan_batches(batches.into_iter().flatten().collect(), 20_000);
    assert!(tight.iter().all(|batch| batch.len() <= 2));
}
#[test]
fn output_updates_preserve_identity_and_concurrent_edit_protection() {
    let mut out = valid_output();
    out["page_proposals"][0]["op"] = json!("update");
    out["page_proposals"][0]["existing_note_id"] = json!("note");
    let entry = NoteIndexEntry {
        note_id: "note".into(),
        rel: "knowledge/methods/existing.md".into(),
        page_type: "method".into(),
        hash: "request-time-hash".into(),
        ..Default::default()
    };
    let result = validate_output(out.clone(), &[input()], std::slice::from_ref(&entry)).unwrap();
    assert_eq!(result.proposals[0].before_hash, "request-time-hash");
    assert!(validate_output(out, &[input()], &[entry.clone(), entry]).is_err());
}
#[tokio::test]
async fn first_page_is_durable_and_repeat_does_not_duplicate_consumption() {
    let db = crate::db::test_helpers::fresh_in_memory_db().await;
    let db = &db.conn;
    let dir = tempdir().unwrap();
    let vault = dir.path().join("vault");
    let state = dir.path().join("state");
    vault::initialize_vault(&vault).unwrap();
    let vault_row = wiki_service::ensure_active_vault(db, &vault.to_string_lossy())
        .await
        .unwrap();
    let md="---\ntype: turn-summary\ncodeg_note_id: memory\n---\n\nA cursor must keep sorting stable while paging.\n";
    fs::write(vault.join("work/turns/source.md"), md).unwrap();
    let job = wiki_service::insert_kind_job(
        db,
        &vault_row.id,
        "wiki_synthesize",
        "first-page",
        None,
        None,
        wiki_service::InsertMode::ReuseTerminal,
    )
    .await
    .unwrap();
    let output = valid_output();
    let llm = crate::wiki::llm::MockWikiLlm::default().with_stage("synthesize", output);
    let result = run_compile_job(db, &job, &llm, &vault, &state)
        .await
        .unwrap();
    assert_eq!(result.outcome, "generated");
    assert_eq!(result.outputs.len(), 1);
    assert_eq!(
        result.outputs[0].content_hash,
        content_hash(&fs::read_to_string(vault.join(&result.outputs[0].path)).unwrap())
    );
    assert!(list_pending_inputs(db, &vault_row.id, &vault)
        .await
        .unwrap()
        .memory_notes
        .is_empty());
}

struct ReadingTestLlm {
    vault: PathBuf,
    calls: std::sync::atomic::AtomicUsize,
    fail_at: usize,
}
#[async_trait::async_trait]
impl WikiLlm for ReadingTestLlm {
    async fn complete_json(
        &self,
        stage: &str,
        input: Value,
    ) -> Result<crate::wiki::llm::WikiLlmRun, WikiLlmError> {
        assert_eq!(stage, "synthesize", "no additional segment-summary calls");
        let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        if call == self.fail_at {
            return Err(WikiLlmError::Failed("test provider disconnected".into()));
        }
        let references = input["source_references"].as_array().unwrap();
        for reference in references {
            assert!(reference.get("content_hash").is_none());
            assert!(reference.get("line_count").is_none());
            fs::read_to_string(self.vault.join(reference["rel"].as_str().unwrap())).unwrap();
        }
        let refs: Vec<_> = references.iter().map(|r| r["rel"].clone()).collect();
        Ok(crate::wiki::llm::WikiLlmRun {
            output: json!({"schema":SYNTHESIZE_CONTRACT_VERSION,"page_proposals":[{
                "proposal_key":"method","op":"create","type":"method","title":"Stable ordering",
                "body":"Stable ordering avoids repeated cursor items.","input_rels":refs
            }],"warnings":[]}),
        })
    }
    fn input_budget(&self) -> u64 {
        150_000
    }
}

#[tokio::test]
async fn later_batch_failure_preserves_outputs_and_retry_consumes_only_remaining_inputs() {
    let db = crate::db::test_helpers::fresh_in_memory_db().await;
    let dir = tempdir().unwrap();
    let vault = dir.path().join("vault");
    let state = dir.path().join("state");
    vault::initialize_vault(&vault).unwrap();
    let row = wiki_service::ensure_active_vault(&db.conn, &vault.to_string_lossy())
        .await
        .unwrap();
    for n in 0..10 {
        fs::write(
            vault.join(format!("work/turns/{n:02}.md")),
            format!(
                "---\ntype: turn-summary\n---\n{}",
                "cursor boundary ".repeat(600)
            ),
        )
        .unwrap();
    }
    let job = wiki_service::insert_kind_job(
        &db.conn,
        &row.id,
        "wiki_synthesize",
        "partial-test",
        None,
        None,
        wiki_service::InsertMode::ReuseTerminal,
    )
    .await
    .unwrap();
    let llm = ReadingTestLlm {
        vault: vault.clone(),
        calls: std::sync::atomic::AtomicUsize::new(0),
        fail_at: 2,
    };
    assert!(run_compile_job(&db.conn, &job, &llm, &vault, &state)
        .await
        .is_err());
    let partial = pipeline::load_job_result(&db.conn, &job.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(partial.outcome, "partial");
    assert_eq!(partial.outputs.len(), 1);
    assert!(!partial.remaining_inputs.is_empty());
    assert_eq!(
        partial.processed_inputs.len() + partial.remaining_inputs.len(),
        10
    );
    let mut retry = wiki_service::get_job_model(&db.conn, &job.id)
        .await
        .unwrap();
    retry.attempt += 1;
    let llm = ReadingTestLlm {
        vault: vault.clone(),
        calls: std::sync::atomic::AtomicUsize::new(0),
        fail_at: usize::MAX,
    };
    let result = run_compile_job(&db.conn, &retry, &llm, &vault, &state)
        .await
        .unwrap();
    assert_eq!(result.outcome, "generated");
    assert_eq!(result.outputs.len(), 2);
    assert_eq!(result.processed_inputs.len(), 10);
    assert!(list_pending_inputs(&db.conn, &row.id, &vault)
        .await
        .unwrap()
        .memory_notes
        .is_empty());
}

#[tokio::test]
async fn long_input_uses_one_agent_run_without_preliminary_segment_summaries() {
    let db = crate::db::test_helpers::fresh_in_memory_db().await;
    let dir = tempdir().unwrap();
    let vault = dir.path().join("vault");
    let state = dir.path().join("state");
    vault::initialize_vault(&vault).unwrap();
    let row = wiki_service::ensure_active_vault(&db.conn, &vault.to_string_lossy())
        .await
        .unwrap();
    fs::write(vault.join("work/turns/long.md"), "中文😀".repeat(12_000)).unwrap();
    let job = wiki_service::insert_kind_job(
        &db.conn,
        &row.id,
        "wiki_synthesize",
        "long-test",
        None,
        None,
        wiki_service::InsertMode::ReuseTerminal,
    )
    .await
    .unwrap();
    let llm = ReadingTestLlm {
        vault: vault.clone(),
        calls: std::sync::atomic::AtomicUsize::new(0),
        fail_at: 2,
    };
    let result = run_compile_job(&db.conn, &job, &llm, &vault, &state)
        .await
        .unwrap();
    assert_eq!(result.outcome, "generated");
    assert_eq!(llm.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn recovery_finalizes_committed_files_once_and_never_revives_deleted_notes() {
    let db = crate::db::test_helpers::fresh_in_memory_db().await;
    let dir = tempdir().unwrap();
    let vault = dir.path().join("vault");
    let state = dir.path().join("state");
    vault::initialize_vault(&vault).unwrap();
    let row = wiki_service::ensure_active_vault(&db.conn, &vault.to_string_lossy())
        .await
        .unwrap();
    let job = wiki_service::insert_kind_job(
        &db.conn,
        &row.id,
        "wiki_synthesize",
        "recovery-test",
        None,
        None,
        wiki_service::InsertMode::ReuseTerminal,
    )
    .await
    .unwrap();
    let mut input = input();
    input.source_ids.clear();
    let validated = validate_output(valid_output(), &[input], &[]).unwrap();
    let mut result = empty_result("generated");
    result.processed_inputs = validated.processed;
    let meta = commit::BatchMetadata {
        vault_id: row.id.clone(),
        attempt: 1,
        batch_id: "batch-0001".into(),
        contract_version: SYNTHESIZE_CONTRACT_VERSION.into(),
        result,
        source_ids_by_note: BTreeMap::new(),
    };
    let committed =
        commit::commit_batch(&vault, &state, &job.id, &validated.proposals, meta).unwrap();
    let path = vault.join(&committed.files[0].rel);
    assert!(path.exists());
    assert!(pipeline::load_job_result(&db.conn, &job.id)
        .await
        .unwrap()
        .is_none());
    let results = recover_batches(&db.conn, &state).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].1.outputs.len(), 1);
    fs::remove_file(&path).unwrap();
    assert!(recover_batches(&db.conn, &state).await.unwrap().is_empty());
    assert!(!path.exists());
    // Simulate losing only the final file-manifest flag after DB commit.
    let mut old = committed;
    old.finalized = false;
    commit::persist_manifest(&state, &old).unwrap();
    recover_batches(&db.conn, &state).await.unwrap();
    assert!(!path.exists());
}

#[test]
fn standard_links_resolve_from_the_output_directory() {
    let mut output = valid_output();
    output["page_proposals"][0]["related_proposal_keys"] = json!(["capability"]);
    let mut second = output["page_proposals"][0].clone();
    second["proposal_key"] = json!("capability");
    second["type"] = json!("capability");
    second["title"] = json!("Method capability");
    second["related_proposal_keys"] = json!([]);
    output["page_proposals"]
        .as_array_mut()
        .unwrap()
        .push(second);
    let validated = validate_output(output, &[input()], &[]).unwrap();
    let method = &validated.proposals[0];
    let capability = &validated.proposals[1];
    assert!(method
        .after
        .contains("[work/turns/source.md](../../work/turns/source.md)"));
    assert!(method
        .after
        .contains(&format!("[capability](../../{})", capability.rel)));
}

#[test]
fn supersede_link_resolves_from_existing_page_directory() {
    let mut output = valid_output();
    output["page_proposals"][0]["op"] = json!("supersede");
    output["page_proposals"][0]["existing_note_id"] = json!("old");
    output["page_proposals"][0]["replacement_note_id"] = json!("replacement");
    let index = vec![
        NoteIndexEntry {
            note_id: "old".into(),
            rel: "knowledge/methods/old.md".into(),
            page_type: "method".into(),
            hash: "before".into(),
            body: "Earlier supported claim.".into(),
            ..Default::default()
        },
        NoteIndexEntry {
            note_id: "replacement".into(),
            rel: "capabilities/replacement.md".into(),
            page_type: "capability".into(),
            ..Default::default()
        },
    ];
    let validated = validate_output(output, &[input()], &index).unwrap();
    assert!(validated.proposals[0]
        .after
        .contains("](../../capabilities/replacement.md)"));
    assert!(validated.proposals[0]
        .after
        .contains("Earlier supported claim."));
}

#[test]
fn synthesis_does_not_generate_source_wrappers() {
    let mut output = valid_output();
    output["page_proposals"][0]["type"] = json!("source");
    assert!(validate_output(output, &[input()], &[]).is_err());
}

#[tokio::test]
async fn recovery_conflict_in_one_job_does_not_block_another_job() {
    let db = crate::db::test_helpers::fresh_in_memory_db().await;
    let dir = tempdir().unwrap();
    let vault = dir.path().join("vault");
    let state = dir.path().join("state");
    vault::initialize_vault(&vault).unwrap();
    let row = wiki_service::ensure_active_vault(&db.conn, &vault.to_string_lossy())
        .await
        .unwrap();
    let mut jobs = Vec::new();
    for n in 0..2 {
        let job = wiki_service::insert_kind_job(
            &db.conn,
            &row.id,
            "wiki_synthesize",
            &format!("recovery-job-{n}"),
            None,
            None,
            wiki_service::InsertMode::ReuseTerminal,
        )
        .await
        .unwrap();
        let mut input = input();
        input.source_ids.clear();
        let validated = validate_output(valid_output(), &[input], &[]).unwrap();
        let mut result = empty_result("generated");
        result.processed_inputs = validated.processed;
        let meta = commit::BatchMetadata {
            vault_id: row.id.clone(),
            attempt: 1,
            batch_id: "batch-0001".into(),
            contract_version: SYNTHESIZE_CONTRACT_VERSION.into(),
            result,
            source_ids_by_note: BTreeMap::new(),
        };
        let committed =
            commit::commit_batch(&vault, &state, &job.id, &validated.proposals, meta).unwrap();
        if n == 0 {
            fs::write(
                vault.join(&committed.files[0].rel),
                "User changed the page after the interrupted commit.",
            )
            .unwrap();
        }
        jobs.push(job.id);
    }
    let results = recover_batches_outcomes(&db.conn, &state).await;
    assert_eq!(results.len(), 2);
    assert!(results
        .iter()
        .any(|(id, result)| id == &jobs[0] && matches!(result, Err(CompileError::Conflict(_)))));
    assert!(results
        .iter()
        .any(|(id, result)| id == &jobs[1] && result.is_ok()));
    assert!(pipeline::load_job_result(&db.conn, &jobs[0])
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        pipeline::load_job_result(&db.conn, &jobs[1])
            .await
            .unwrap()
            .unwrap()
            .outputs
            .len(),
        1
    );
}
