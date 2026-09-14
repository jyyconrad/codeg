//! Bundled Codeg Agent skills extracted to `codeg-agent/skills/.system`.

use std::fs;
use std::io;
use std::path::PathBuf;

use rig::agent::RequestPatch;
use rig::completion::Document;

use crate::paths::codeg_agent_dir;

pub const USING_PLAN_EXPLORE_ID: &str = "using-plan-explore";
pub const USING_PLAN_EXPLORE_DOC_ID: &str = "codeg-using-plan-explore";
pub const WIKI_TURN_SUMMARY_ID: &str = "wiki-turn-summary";
pub const WIKI_SESSION_ROLLUP_ID: &str = "wiki-session-rollup";
pub const WIKI_SYNTHESIZE_ID: &str = "wiki-synthesize";

const USING_PLAN_EXPLORE_SKILL: &str =
    include_str!("../../agent-skills/using-plan-explore/SKILL.md");
const WIKI_TURN_SUMMARY_SKILL: &str = include_str!("../../agent-skills/wiki-turn-summary/SKILL.md");
const WIKI_SESSION_ROLLUP_SKILL: &str =
    include_str!("../../agent-skills/wiki-session-rollup/SKILL.md");
const WIKI_SYNTHESIZE_SKILL: &str = include_str!("../../agent-skills/wiki-synthesize/SKILL.md");

pub fn system_skills_dir() -> PathBuf {
    codeg_agent_dir().join("skills").join(".system")
}

pub fn using_plan_explore_dir() -> PathBuf {
    system_skills_dir().join(USING_PLAN_EXPLORE_ID)
}

fn install_bundled_skill(id: &str, body: &str) -> io::Result<PathBuf> {
    let dir = system_skills_dir().join(id);
    fs::create_dir_all(&dir)?;
    let path = dir.join("SKILL.md");
    let current = fs::read_to_string(&path).unwrap_or_default();
    if current != body {
        fs::write(&path, body)?;
    }
    Ok(path)
}

/// Write bundled skills under `skills/.system`. Idempotent when content matches.
///
/// Wiki turn/session/synthesize live on this same `.system` path so WikiWorker
/// can load them later. They set `disable-model-invocation` and are not
/// attached as general Codeg Agent extra context.
pub fn ensure_installed() -> io::Result<PathBuf> {
    install_bundled_skill(WIKI_TURN_SUMMARY_ID, WIKI_TURN_SUMMARY_SKILL)?;
    install_bundled_skill(WIKI_SESSION_ROLLUP_ID, WIKI_SESSION_ROLLUP_SKILL)?;
    install_bundled_skill(WIKI_SYNTHESIZE_ID, WIKI_SYNTHESIZE_SKILL)?;
    install_bundled_skill(USING_PLAN_EXPLORE_ID, USING_PLAN_EXPLORE_SKILL)
}

pub fn using_plan_explore_document() -> Document {
    Document {
        id: USING_PLAN_EXPLORE_DOC_ID.to_string(),
        text: USING_PLAN_EXPLORE_SKILL.to_string(),
        additional_props: Default::default(),
    }
}

pub fn attach_using_plan_explore(patch: RequestPatch) -> RequestPatch {
    patch.context(using_plan_explore_document())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::context::budget::per_call_patch;

    fn system_skill_path(home: &std::path::Path, id: &str) -> PathBuf {
        home.join("codeg-agent")
            .join("skills")
            .join(".system")
            .join(id)
            .join("SKILL.md")
    }

    #[test]
    fn bundled_skill_has_required_frontmatter() {
        assert!(USING_PLAN_EXPLORE_SKILL.contains("name: using-plan-explore"));
        assert!(USING_PLAN_EXPLORE_SKILL.contains("description: Use when"));
        assert!(USING_PLAN_EXPLORE_SKILL.contains("enter_plan_mode"));
        assert!(USING_PLAN_EXPLORE_SKILL.contains("subagent"));
        assert!(USING_PLAN_EXPLORE_SKILL.contains("update_plan"));
        assert!(USING_PLAN_EXPLORE_SKILL.contains("Explore report ready"));
    }

    #[test]
    fn ensure_installed_writes_under_codeg_home() {
        let tmp = tempfile::tempdir().expect("temp");
        let home = tmp.path().join("home");
        temp_env::with_vars(
            [
                ("HOME", Some(tmp.path())),
                ("CODEG_HOME", Some(home.as_path())),
                ("CODEG_DATA_DIR", None::<&std::path::Path>),
            ],
            || {
                let path = ensure_installed().expect("install");
                assert_eq!(path, system_skill_path(&home, USING_PLAN_EXPLORE_ID));
                let body = fs::read_to_string(&path).expect("read");
                assert_eq!(body, USING_PLAN_EXPLORE_SKILL);

                let turn = system_skill_path(&home, WIKI_TURN_SUMMARY_ID);
                let session = system_skill_path(&home, WIKI_SESSION_ROLLUP_ID);
                let synthesize = system_skill_path(&home, WIKI_SYNTHESIZE_ID);
                assert_eq!(
                    fs::read_to_string(&turn).expect("read turn"),
                    WIKI_TURN_SUMMARY_SKILL
                );
                assert_eq!(
                    fs::read_to_string(&session).expect("read session"),
                    WIKI_SESSION_ROLLUP_SKILL
                );
                assert_eq!(
                    fs::read_to_string(&synthesize).expect("read synthesize"),
                    WIKI_SYNTHESIZE_SKILL
                );
                ensure_installed().expect("idempotent");
                assert_eq!(
                    fs::read_to_string(&turn).expect("reread turn"),
                    WIKI_TURN_SUMMARY_SKILL
                );
                assert_eq!(
                    fs::read_to_string(&session).expect("reread session"),
                    WIKI_SESSION_ROLLUP_SKILL
                );
                assert_eq!(
                    fs::read_to_string(&synthesize).expect("reread synthesize"),
                    WIKI_SYNTHESIZE_SKILL
                );
            },
        );
    }

    #[test]
    fn extra_context_attaches_the_skill() {
        let patch = attach_using_plan_explore(per_call_patch(Vec::new(), Some(128)));
        assert_eq!(patch.extra_context.len(), 1);
        assert_eq!(patch.extra_context[0].id, USING_PLAN_EXPLORE_DOC_ID);
        assert!(patch.extra_context[0].text.contains("enter_plan_mode"));
    }

    #[test]
    fn catalog_lists_the_builtin_skill_after_install() {
        let tmp = tempfile::tempdir().expect("temp");
        let home = tmp.path().join("home");
        temp_env::with_vars(
            [
                ("HOME", Some(tmp.path())),
                ("CODEG_HOME", Some(home.as_path())),
                ("CODEG_DATA_DIR", None::<&std::path::Path>),
            ],
            || {
                ensure_installed().expect("install");
                let catalog = crate::agent::tools::SkillCatalog::load(
                    crate::models::agent::AgentType::CodegAgent,
                    None,
                );
                let item = catalog
                    .get(USING_PLAN_EXPLORE_ID)
                    .expect("builtin skill in catalog");
                assert!(
                    item.description
                        .as_deref()
                        .unwrap_or("")
                        .contains("enter plan mode"),
                    "{:?}",
                    item.description
                );
                let section = catalog.preamble_section().expect("section");
                assert!(section.contains("`using-plan-explore`"), "{section}");
                assert!(
                    catalog.get(WIKI_TURN_SUMMARY_ID).is_none(),
                    "wiki-turn-summary is WikiWorker-only"
                );
                assert!(
                    catalog.get(WIKI_SESSION_ROLLUP_ID).is_none(),
                    "wiki-session-rollup is WikiWorker-only"
                );
                assert!(
                    catalog.get(WIKI_SYNTHESIZE_ID).is_none(),
                    "wiki-synthesize is WikiWorker-only"
                );
                assert!(!section.contains("`wiki-ingest`"), "{section}");
                assert!(!section.contains("`wiki-compile`"), "{section}");
                assert!(!section.contains("`wiki-turn-summary`"), "{section}");
                assert!(!section.contains("`wiki-session-rollup`"), "{section}");
                assert!(!section.contains("`wiki-synthesize`"), "{section}");
            },
        );
    }
}
