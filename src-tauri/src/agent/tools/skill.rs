//! Skill catalog and tool: `skill_storage_spec` + `list_skills_from_dir`.
//!
//! Discovery stays in `commands/acp.rs`. This module does not scan SKILL.md
//! directories itself.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::json;

use super::NativeToolCtx;
use crate::acp::types::{AgentSkillItem, AgentSkillScope};
use crate::agent::context::OutputLocator;
use crate::commands::acp::{
    list_skills_from_dir, parse_frontmatter_scalar, scoped_skill_dirs, skill_content_path,
    skill_storage_spec,
};
use crate::models::agent::AgentType;

/// Deduped skills from the existing storage spec. Catalog text is the
/// description-bearing subset; the tool locates the same items by name.
#[derive(Clone, Debug, Default)]
pub struct SkillCatalog {
    items: Arc<Vec<AgentSkillItem>>,
}

impl SkillCatalog {
    pub fn load(agent_type: AgentType, workspace_path: Option<&str>) -> Self {
        Self::from_items(collect_skill_items(agent_type, workspace_path))
    }

    pub fn from_items(items: Vec<AgentSkillItem>) -> Self {
        let usable: Vec<AgentSkillItem> = items
            .into_iter()
            .filter(|item| !disable_model_invocation(&skill_md_path(item)))
            .collect();
        Self {
            items: Arc::new(usable),
        }
    }

    pub fn get(&self, name: &str) -> Option<&AgentSkillItem> {
        let needle = name.trim();
        if needle.is_empty() {
            return None;
        }
        self.items
            .iter()
            .find(|item| item.id == needle || item.name == needle)
    }

    /// Full SKILL.md body for a named skill, if the file can be read.
    pub fn skill_body(&self, name: &str) -> Option<String> {
        let item = self.get(name)?;
        fs::read_to_string(skill_md_path(item)).ok()
    }

    pub fn items(&self) -> &[AgentSkillItem] {
        &self.items
    }

    /// Name + description lines for the preamble. Empty descriptions omitted.
    pub fn preamble_section(&self) -> Option<String> {
        let mut lines = Vec::new();
        for item in self.items.iter() {
            let Some(description) = item
                .description
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            lines.push(format!("- `{}`: {description}", item.name));
        }
        if lines.is_empty() {
            return None;
        }
        let mut section = String::from(
            "Available skills (call the skill tool with `name` to load SKILL.md into context):\n",
        );
        section.push_str(&lines.join("\n"));
        Some(section)
    }
}

fn collect_skill_items(agent_type: AgentType, workspace_path: Option<&str>) -> Vec<AgentSkillItem> {
    let Some(spec) = skill_storage_spec(agent_type) else {
        return Vec::new();
    };
    let mut by_id: BTreeMap<String, AgentSkillItem> = BTreeMap::new();
    if let Ok(dirs) = scoped_skill_dirs(agent_type, AgentSkillScope::Global, None) {
        insert_listed(&mut by_id, &dirs, spec.kind, AgentSkillScope::Global, false);
    }
    if let Some(workspace) = workspace_path.map(str::trim).filter(|p| !p.is_empty()) {
        if let Ok(dirs) = scoped_skill_dirs(agent_type, AgentSkillScope::Project, Some(workspace)) {
            insert_listed(&mut by_id, &dirs, spec.kind, AgentSkillScope::Project, true);
        }
    }
    by_id.into_values().collect()
}

fn insert_listed(
    by_id: &mut BTreeMap<String, AgentSkillItem>,
    dirs: &[PathBuf],
    kind: crate::commands::acp::SkillStorageKind,
    scope: AgentSkillScope,
    overwrite: bool,
) {
    for dir in dirs {
        let listed = match list_skills_from_dir(scope, dir, kind) {
            Ok(items) => items,
            Err(err) => {
                tracing::warn!(
                    path = %dir.display(),
                    error = %err,
                    "failed to list skills from directory"
                );
                continue;
            }
        };
        for item in listed {
            if overwrite {
                by_id.insert(item.id.clone(), item);
            } else {
                by_id.entry(item.id.clone()).or_insert(item);
            }
        }
    }
}

fn skill_md_path(item: &AgentSkillItem) -> PathBuf {
    skill_content_path(item.layout, Path::new(&item.path))
}

/// `disable-model-invocation` from the first 4KiB via `parse_frontmatter_scalar`.
/// Missing key → not disabled (same as Settings: listed skills are usable).
fn disable_model_invocation(content_path: &Path) -> bool {
    let mut file = match fs::File::open(content_path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut buf = [0u8; 4096];
    let n = match file.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return false,
    };
    let Ok(head) = std::str::from_utf8(&buf[..n]) else {
        return false;
    };
    let mut lines = head.lines();
    if lines.next().map(str::trim) != Some("---") {
        return false;
    }
    for line in lines {
        let trimmed_end = line.trim_end();
        if trimmed_end == "---" || trimmed_end == "..." {
            break;
        }
        if line.starts_with(|c: char| c.is_whitespace()) {
            continue;
        }
        if let Some(rest) = line.strip_prefix("disable-model-invocation:") {
            if let Some(val) = parse_frontmatter_scalar(rest) {
                return matches!(val.to_ascii_lowercase().as_str(), "true" | "yes" | "1");
            }
        }
    }
    false
}

/// Session-scoped set of skills already injected into model context.
#[derive(Clone, Debug, Default)]
pub struct LoadedSkills {
    names: BTreeSet<String>,
}

impl LoadedSkills {
    pub fn shared() -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self::default()))
    }

    pub fn shared_with(names: impl IntoIterator<Item = impl Into<String>>) -> Arc<Mutex<Self>> {
        let mut loaded = Self::default();
        for name in names {
            loaded.insert(&name.into());
        }
        Arc::new(Mutex::new(loaded))
    }

    pub fn contains(&self, name: &str) -> bool {
        let needle = name.trim();
        !needle.is_empty() && self.names.contains(needle)
    }

    pub fn insert(&mut self, name: &str) {
        let needle = name.trim();
        if !needle.is_empty() {
            self.names.insert(needle.to_string());
        }
    }
}

/// Drop a leading YAML `---` / `...` frontmatter block. Body is unchanged
/// when the file has no frontmatter.
pub(crate) fn strip_yaml_frontmatter(content: &str) -> &str {
    let Some(first) = content.lines().next() else {
        return content;
    };
    if first.trim_end() != "---" {
        return content;
    }
    let mut consumed = 0usize;
    let mut started = false;
    for line in content.split_inclusive('\n') {
        consumed += line.len();
        if !started {
            started = true;
            continue;
        }
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" || trimmed == "..." {
            return content.get(consumed..).unwrap_or("").trim_start();
        }
    }
    content
}

#[derive(Clone)]
pub struct SkillTool {
    ctx: NativeToolCtx,
    catalog: SkillCatalog,
}

impl SkillTool {
    pub fn new(ctx: NativeToolCtx, catalog: SkillCatalog) -> Self {
        Self { ctx, catalog }
    }
}

#[derive(Debug, Deserialize)]
pub struct SkillArgs {
    pub name: String,
}

impl Tool for SkillTool {
    const NAME: &'static str = "skill";
    type Args = SkillArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Load a specialized skill when the task matches one listed in the system \
         prompt. Injects the skill's SKILL.md instructions (frontmatter stripped) \
         and sampled files from the skill directory. Relative paths (scripts/, \
         reference/) are relative to that directory. If the skill is already in \
         context, returns that it is loaded instead of dumping the body again."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Skill id or name from the catalog" }
            },
            "required": ["name"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let raw = json!({ "name": args.name });
        let mut fact = self.ctx.begin(Self::NAME, raw).await?;
        match load_skill(&self.catalog, &self.ctx.loaded_skills, &args.name) {
            Ok(loaded) => {
                fact.truncated = loaded.truncated;
                fact.output_locator = Some(loaded.locator);
                self.ctx.finish_ok(fact, loaded.presentation).await
            }
            Err(err) => Err(self.ctx.finish_err(fact, err).await),
        }
    }
}

struct LoadedSkill {
    presentation: String,
    truncated: bool,
    locator: OutputLocator,
}

fn load_skill(
    catalog: &SkillCatalog,
    loaded: &Mutex<LoadedSkills>,
    name: &str,
) -> Result<LoadedSkill, ToolExecutionError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(
            ToolExecutionError::invalid_args("skill name must not be empty")
                .with_model_feedback("skill name must not be empty"),
        );
    }
    let item = catalog.get(trimmed).ok_or_else(|| {
        let available = catalog
            .items()
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>();
        let listed = if available.is_empty() {
            "none".to_string()
        } else {
            available.join(", ")
        };
        let msg = format!("Skill \"{trimmed}\" not found. Available skills: {listed}");
        ToolExecutionError::not_found(msg.clone()).with_model_feedback(msg)
    })?;
    let path = skill_md_path(item);
    let already = {
        let guard = loaded.lock().expect("loaded skills");
        guard.contains(&item.id) || guard.contains(&item.name) || guard.contains(trimmed)
    };
    if already {
        return Ok(already_loaded_skill(&item.name, &path));
    }
    let content = read_skill_text(&path)?;
    let files = sample_skill_files(&path, 10);
    let loaded_skill = present_skill_markdown(&item.name, &path, &content, &files);
    if let Ok(mut guard) = loaded.lock() {
        guard.insert(&item.id);
        guard.insert(&item.name);
    }
    Ok(loaded_skill)
}

fn read_skill_text(path: &Path) -> Result<String, ToolExecutionError> {
    fs::read_to_string(path).map_err(|_| {
        ToolExecutionError::not_found(format!("skill file not found: {}", path.display()))
            .with_model_feedback(format!("skill file not found: {}", path.display()))
    })
}

fn skill_base_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(path)
}

fn already_loaded_skill(name: &str, path: &Path) -> LoadedSkill {
    let base = skill_base_dir(path);
    LoadedSkill {
        presentation: format!(
            "Skill `{name}` is already loaded in the context.\n\
             Base directory for this skill: {}\n\
             Relative paths in this skill (e.g., scripts/, reference/) are relative to this base directory.",
            base.display()
        ),
        truncated: false,
        locator: OutputLocator {
            path: Some(path.to_string_lossy().into_owned()),
            line: Some(1),
        },
    }
}

fn sample_skill_files(skill_md: &Path, max: usize) -> Vec<PathBuf> {
    if max == 0 {
        return Vec::new();
    }
    if skill_md.file_name().and_then(|n| n.to_str()) != Some("SKILL.md") {
        return Vec::new();
    }
    let dir = skill_base_dir(skill_md);
    let mut out = Vec::new();
    let mut queue = VecDeque::from([dir.to_path_buf()]);
    while let Some(cur) = queue.pop_front() {
        let Ok(entries) = fs::read_dir(&cur) else {
            continue;
        };
        let mut ents: Vec<_> = entries.filter_map(|e| e.ok()).collect();
        ents.sort_by_key(|e| e.file_name());
        for ent in ents {
            if out.len() >= max {
                return out;
            }
            let name = ent.file_name();
            if name == ".git" {
                continue;
            }
            let path = ent.path();
            let Ok(ft) = ent.file_type() else {
                continue;
            };
            if ft.is_dir() {
                queue.push_back(path);
            } else if ft.is_file() && name != "SKILL.md" {
                out.push(path);
            }
        }
    }
    out
}

fn present_skill_markdown(
    name: &str,
    path: &Path,
    content: &str,
    files: &[PathBuf],
) -> LoadedSkill {
    let body = strip_yaml_frontmatter(content).trim();
    let base = skill_base_dir(path);
    let mut blocks = vec![
        format!("<skill_content name=\"{name}\">"),
        format!("# Skill: {name}"),
        String::new(),
        body.to_string(),
        String::new(),
        format!("Base directory for this skill: {}", base.display()),
        "Relative paths in this skill (e.g., scripts/, reference/) are relative to this base directory."
            .to_string(),
        "Note: file list is sampled.".to_string(),
        String::new(),
        "<skill_files>".to_string(),
    ];
    for file in files {
        blocks.push(format!("<file>{}</file>", file.display()));
    }
    blocks.push("</skill_files>".to_string());
    blocks.push("</skill_content>".to_string());
    LoadedSkill {
        presentation: blocks.join("\n"),
        truncated: false,
        locator: OutputLocator {
            path: Some(path.to_string_lossy().into_owned()),
            line: Some(1),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::context::{CallIdentity, ToolOutcome, MAX_TOOL_PRESENTATION_BYTES};
    use crate::agent::tools::test_tool_ctx;
    use crate::commands::acp::{list_skills_from_dir, SkillStorageKind};
    use rig::tool::Tool;

    fn write_skill(dir: &Path, id: &str, body: &str) -> PathBuf {
        let skill_dir = dir.join(id);
        fs::create_dir_all(&skill_dir).expect("skill dir");
        let path = skill_dir.join("SKILL.md");
        fs::write(&path, body).expect("write SKILL.md");
        path
    }

    #[test]
    fn catalog_uses_list_skills_from_dir_not_a_new_scanner() {
        let root = tempfile::tempdir().expect("temp");
        write_skill(
            root.path(),
            "demo",
            "---\nname: demo\ndescription: listed skill\n---\nbody\n",
        );
        fs::create_dir_all(root.path().join("empty")).expect("empty dir");
        fs::write(root.path().join("flat.md"), "---\ndescription: flat\n---\n").expect("flat");

        let listed = list_skills_from_dir(
            AgentSkillScope::Global,
            root.path(),
            SkillStorageKind::SkillDirectoryOnly,
        )
        .expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "demo");
        assert_eq!(listed[0].description.as_deref(), Some("listed skill"));

        let catalog = SkillCatalog::from_items(listed);
        let section = catalog.preamble_section().expect("catalog");
        assert!(section.contains("`demo`"), "{section}");
        assert!(section.contains("listed skill"), "{section}");
        assert!(!section.contains("flat"), "{section}");
        assert!(!section.contains("empty"), "{section}");
    }

    #[test]
    fn disable_model_invocation_uses_parse_frontmatter_scalar() {
        assert_eq!(parse_frontmatter_scalar(" true").as_deref(), Some("true"));
        let root = tempfile::tempdir().expect("temp");
        write_skill(
            root.path(),
            "hidden",
            "---\nname: hidden\ndescription: secret\ndisable-model-invocation: true\n---\nnope\n",
        );
        write_skill(
            root.path(),
            "visible",
            "---\nname: visible\ndescription: ok\n---\nbody\n",
        );
        let listed = list_skills_from_dir(
            AgentSkillScope::Global,
            root.path(),
            SkillStorageKind::SkillDirectoryOnly,
        )
        .expect("list");
        let catalog = SkillCatalog::from_items(listed);
        assert!(catalog.get("hidden").is_none());
        assert!(catalog.get("visible").is_some());
        let section = catalog.preamble_section().expect("section");
        assert!(section.contains("`visible`"), "{section}");
        assert!(!section.contains("hidden"), "{section}");
    }

    #[test]
    fn skill_presentation_strips_frontmatter_and_keeps_full_body() {
        let mut body = String::from("---\nname: big\ndescription: large\n---\n");
        for i in 0..2500 {
            body.push_str(&format!("line-{i:04} {}\n", "x".repeat(24)));
        }
        assert!(body.len() > MAX_TOOL_PRESENTATION_BYTES);

        let path = PathBuf::from("/tmp/codeg-agent-skills/big/SKILL.md");
        let extra = PathBuf::from("/tmp/codeg-agent-skills/big/scripts/run.sh");
        let loaded = present_skill_markdown("big", &path, &body, std::slice::from_ref(&extra));
        assert!(!loaded.truncated);
        assert!(
            loaded.presentation.contains("<skill_content name=\"big\">"),
            "{}",
            loaded.presentation
        );
        assert!(
            loaded.presentation.contains("# Skill: big"),
            "{}",
            loaded.presentation
        );
        assert!(
            loaded
                .presentation
                .contains("Base directory for this skill: /tmp/codeg-agent-skills/big"),
            "{}",
            loaded.presentation
        );
        assert!(
            loaded
                .presentation
                .contains("<file>/tmp/codeg-agent-skills/big/scripts/run.sh</file>"),
            "{}",
            loaded.presentation
        );
        assert!(
            loaded.presentation.contains("line-0000"),
            "{}",
            loaded.presentation
        );
        assert!(
            loaded.presentation.contains("line-2499"),
            "{}",
            loaded.presentation
        );
        assert!(
            !loaded.presentation.contains("description: large"),
            "YAML frontmatter must not be in the body:\n{}",
            loaded.presentation
        );
        assert!(
            !loaded.presentation.contains("pass offset="),
            "{}",
            loaded.presentation
        );
        assert_eq!(
            loaded.locator.path.as_deref(),
            Some("/tmp/codeg-agent-skills/big/SKILL.md")
        );
    }

    #[tokio::test]
    async fn skill_tool_reads_catalog_item_outside_fs_policy() {
        let launch = tempfile::tempdir().expect("cwd");
        let skills = tempfile::tempdir().expect("skills");
        let md = write_skill(
            skills.path(),
            "outside",
            "---\nname: outside\ndescription: from catalog\n---\nhello skill\n",
        );
        let script = md.parent().expect("dir").join("scripts").join("run.sh");
        fs::create_dir_all(script.parent().expect("scripts")).expect("scripts dir");
        fs::write(&script, "echo hi\n").expect("script");
        let listed = list_skills_from_dir(
            AgentSkillScope::Global,
            skills.path(),
            SkillStorageKind::SkillDirectoryOnly,
        )
        .expect("list");
        let catalog = SkillCatalog::from_items(listed);
        let ctx = test_tool_ctx(launch.path(), "skill", "call_s");
        let mut tctx = ToolContext::new();
        let out = SkillTool::new(ctx.clone(), catalog)
            .call(
                &mut tctx,
                SkillArgs {
                    name: "outside".into(),
                },
            )
            .await
            .expect("skill");
        assert!(out.contains("hello skill"), "{out}");
        assert!(out.contains("<skill_content name=\"outside\">"), "{out}");
        assert!(
            out.contains(&format!("<file>{}</file>", script.display())),
            "{out}"
        );
        assert!(
            out.contains(&format!(
                "Base directory for this skill: {}",
                md.parent().expect("skill dir").display()
            )),
            "{out}"
        );
        let fact = ctx
            .recorder
            .store()
            .lock()
            .expect("store")
            .fact("call_s")
            .cloned();
        assert_eq!(
            fact.as_ref().and_then(|f| f.outcome),
            Some(ToolOutcome::Success)
        );
        assert_eq!(
            fact.and_then(|f| f.output_locator.and_then(|l| l.path)),
            Some(md.to_string_lossy().into_owned())
        );
    }

    #[tokio::test]
    async fn skill_tool_returns_full_body_then_already_loaded() {
        let launch = tempfile::tempdir().expect("cwd");
        let skills = tempfile::tempdir().expect("skills");
        let mut body = String::from("---\nname: big\ndescription: large\n---\n# Big skill\n");
        for i in 0..2500 {
            body.push_str(&format!("line-{i:04} {}\n", "x".repeat(24)));
        }
        let _md = write_skill(skills.path(), "big", &body);
        let listed = list_skills_from_dir(
            AgentSkillScope::Global,
            skills.path(),
            SkillStorageKind::SkillDirectoryOnly,
        )
        .expect("list");
        let catalog = SkillCatalog::from_items(listed);
        let ctx = test_tool_ctx(launch.path(), "skill", "call_s");
        let mut tctx = ToolContext::new();
        let out = SkillTool::new(ctx.clone(), catalog.clone())
            .call(&mut tctx, SkillArgs { name: "big".into() })
            .await
            .expect("skill");
        assert!(out.contains("<skill_content name=\"big\">"), "{out}");
        assert!(out.contains("# Skill: big"), "{out}");
        assert!(out.contains("Base directory for this skill:"), "{out}");
        assert!(out.contains("# Big skill"), "{out}");
        assert!(out.contains("line-2499"), "{out}");
        assert!(!out.contains("name: big"), "{out}");
        assert!(!out.contains("pass offset="), "{out}");
        let fact = ctx
            .recorder
            .store()
            .lock()
            .expect("store")
            .fact("call_s")
            .cloned()
            .expect("fact");
        assert!(!fact.truncated);

        ctx.identity.set(CallIdentity {
            turn_id: 1,
            turn_key: "s:1".into(),
            tool_call_id: "call_s2".into(),
            function_name: "skill".into(),
        });
        let again = SkillTool::new(ctx, catalog)
            .call(&mut tctx, SkillArgs { name: "big".into() })
            .await
            .expect("already loaded");
        assert!(
            again.contains("already loaded"),
            "repeat load must not dump the body again: {again}"
        );
        assert!(again.contains("Base directory for this skill:"), "{again}");
        assert!(!again.contains("line-2499"), "{again}");
    }

    #[tokio::test]
    async fn skill_tool_unknown_name_fails() {
        let dir = tempfile::tempdir().expect("cwd");
        let ctx = test_tool_ctx(dir.path(), "skill", "call_s");
        ctx.identity.set(CallIdentity {
            turn_id: 1,
            turn_key: "s:1".into(),
            tool_call_id: "call_s".into(),
            function_name: "skill".into(),
        });
        let mut tctx = ToolContext::new();
        let err = SkillTool::new(ctx, SkillCatalog::default())
            .call(
                &mut tctx,
                SkillArgs {
                    name: "missing".into(),
                },
            )
            .await
            .expect_err("missing");
        assert!(
            err.model_feedback()
                .unwrap_or_default()
                .contains("Skill \"missing\" not found"),
            "{err:?}"
        );
    }

    #[test]
    fn load_uses_skill_storage_spec_dirs() {
        let tmp = tempfile::tempdir().expect("temp");
        let home = tmp.path().join("home");
        let ws = tmp.path().join("ws");
        temp_env::with_vars(
            [
                ("HOME", Some(tmp.path())),
                ("CODEG_HOME", Some(home.as_path())),
                ("CODEG_DATA_DIR", None::<&std::path::Path>),
            ],
            || {
                let global = crate::paths::codeg_agent_dir().join("skills");
                write_skill(
                    &global,
                    "global-skill",
                    "---\ndescription: from global\n---\nG\n",
                );
                let project = ws.join(".codeg").join("skills");
                write_skill(
                    &project,
                    "project-skill",
                    "---\ndescription: from project\n---\nP\n",
                );
                write_skill(
                    &project,
                    "global-skill",
                    "---\ndescription: project wins\n---\nX\n",
                );
                let catalog = SkillCatalog::load(AgentType::CodegAgent, Some(ws.to_str().unwrap()));
                assert_eq!(
                    catalog.get("project-skill").map(|i| i.scope),
                    Some(AgentSkillScope::Project)
                );
                assert_eq!(
                    catalog
                        .get("global-skill")
                        .and_then(|i| i.description.as_deref()),
                    Some("project wins")
                );
                let section = catalog.preamble_section().expect("section");
                assert!(section.contains("from project"), "{section}");
                assert!(section.contains("project wins"), "{section}");
                assert!(!section.contains("from global"), "{section}");
            },
        );
    }
}
