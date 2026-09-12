//! Bundled Codeg Agent skills extracted to `codeg-agent/skills/.system`.

use std::fs;
use std::io;
use std::path::PathBuf;

use rig::agent::RequestPatch;
use rig::completion::Document;

use crate::paths::codeg_agent_dir;

pub const USING_PLAN_EXPLORE_ID: &str = "using-plan-explore";
pub const USING_PLAN_EXPLORE_DOC_ID: &str = "codeg-using-plan-explore";
pub const WIKI_INGEST_ID: &str = "wiki-ingest";
pub const WIKI_COMPILE_ID: &str = "wiki-compile";

const USING_PLAN_EXPLORE_SKILL: &str =
    include_str!("../../agent-skills/using-plan-explore/SKILL.md");
const WIKI_INGEST_SKILL: &str = include_str!("../../agent-skills/wiki-ingest/SKILL.md");
const WIKI_COMPILE_SKILL: &str = include_str!("../../agent-skills/wiki-compile/SKILL.md");

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
/// Wiki ingest/compile live on this same `.system` path so WikiWorker can load
/// them later. They set `disable-model-invocation` and are not attached as
/// general Codeg Agent extra context.
pub fn ensure_installed() -> io::Result<PathBuf> {
    install_bundled_skill(WIKI_INGEST_ID, WIKI_INGEST_SKILL)?;
    install_bundled_skill(WIKI_COMPILE_ID, WIKI_COMPILE_SKILL)?;
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
    fn wiki_ingest_skill_has_required_frontmatter_and_contract() {
        assert!(WIKI_INGEST_SKILL.contains("name: wiki-ingest"));
        assert!(WIKI_INGEST_SKILL.contains(
            "description: Use when summarizing a frozen wiki source snapshot into a one-line source summary and optional topic suggestions. Never rewrite evidence."
        ));
        assert!(WIKI_INGEST_SKILL.contains("Do not reconstruct"));
        assert!(WIKI_INGEST_SKILL.contains("host schema"));
        assert!(WIKI_INGEST_SKILL.contains("disable-model-invocation: true"));
    }

    #[test]
    fn wiki_compile_skill_has_required_frontmatter_and_contract() {
        assert!(WIKI_COMPILE_SKILL.contains("name: wiki-compile"));
        assert!(WIKI_COMPILE_SKILL.contains(
            "description: Use when compiling wiki sources into personal work notes and capability notes. Process only the host manifest."
        ));
        assert!(WIKI_COMPILE_SKILL.contains("related≠same"));
        assert!(WIKI_COMPILE_SKILL.contains("knowledge_only"));
        assert!(WIKI_COMPILE_SKILL.contains("host validates and commits"));
        assert!(WIKI_COMPILE_SKILL.contains("disable-model-invocation: true"));
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

                let ingest = system_skill_path(&home, WIKI_INGEST_ID);
                let compile = system_skill_path(&home, WIKI_COMPILE_ID);
                assert_eq!(
                    fs::read_to_string(&ingest).expect("read ingest"),
                    WIKI_INGEST_SKILL
                );
                assert_eq!(
                    fs::read_to_string(&compile).expect("read compile"),
                    WIKI_COMPILE_SKILL
                );
                ensure_installed().expect("idempotent");
                assert_eq!(
                    fs::read_to_string(&ingest).expect("reread ingest"),
                    WIKI_INGEST_SKILL
                );
                assert_eq!(
                    fs::read_to_string(&compile).expect("reread compile"),
                    WIKI_COMPILE_SKILL
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
                    catalog.get(WIKI_INGEST_ID).is_none(),
                    "wiki-ingest is WikiWorker-only"
                );
                assert!(
                    catalog.get(WIKI_COMPILE_ID).is_none(),
                    "wiki-compile is WikiWorker-only"
                );
                assert!(!section.contains("`wiki-ingest`"), "{section}");
                assert!(!section.contains("`wiki-compile`"), "{section}");
            },
        );
    }
}
