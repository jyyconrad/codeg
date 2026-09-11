//! Skill catalog and tool: `skill_storage_spec` + `list_skills_from_dir`.
//!
//! Discovery stays in `commands/acp.rs`. This module does not scan SKILL.md
//! directories itself.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::json;

use super::NativeToolCtx;
use crate::acp::types::{AgentSkillItem, AgentSkillScope};
use crate::agent::context::{OutputLocator, MAX_TOOL_PRESENTATION_BYTES};
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
        let mut section =
            String::from("Available skills (call the skill tool with `name` to load SKILL.md):\n");
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
        "Load a skill by name from the session catalog. Returns SKILL.md with the \
         real file path and line range; use read_file with offset to continue if truncated."
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
        match load_skill(&self.catalog, &args.name) {
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

fn load_skill(catalog: &SkillCatalog, name: &str) -> Result<LoadedSkill, ToolExecutionError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(
            ToolExecutionError::invalid_args("skill name must not be empty")
                .with_model_feedback("skill name must not be empty"),
        );
    }
    let item = catalog.get(trimmed).ok_or_else(|| {
        ToolExecutionError::not_found(format!("skill not found: {trimmed}"))
            .with_model_feedback(format!("skill not found: {trimmed}"))
    })?;
    let path = skill_md_path(item);
    let content = read_skill_text(&path)?;
    Ok(present_skill_markdown(
        &path,
        &content,
        MAX_TOOL_PRESENTATION_BYTES,
    ))
}

fn read_skill_text(path: &Path) -> Result<String, ToolExecutionError> {
    let file = fs::File::open(path).map_err(|_| {
        ToolExecutionError::not_found(format!("skill file not found: {}", path.display()))
            .with_model_feedback(format!("skill file not found: {}", path.display()))
    })?;
    let mut limited = file.take(MAX_TOOL_PRESENTATION_BYTES as u64 + 4096);
    let mut buf = String::new();
    limited.read_to_string(&mut buf).map_err(|_| {
        ToolExecutionError::invalid_args(format!(
            "skill file is not valid UTF-8: {}",
            path.display()
        ))
        .with_model_feedback(format!("skill file is not valid UTF-8: {}", path.display()))
    })?;
    Ok(buf)
}

fn present_skill_markdown(path: &Path, content: &str, max_bytes: usize) -> LoadedSkill {
    let (body, truncated) = truncate_at_line_boundary(content, max_bytes);
    let shown_lines = if body.is_empty() {
        0
    } else {
        body.lines().count() as u32
    };
    let start = 1u32;
    let end = if shown_lines == 0 {
        0
    } else {
        start.saturating_add(shown_lines.saturating_sub(1))
    };
    let next = end.saturating_add(1).max(1);
    let mut presentation = format!("# {} (lines {start}-{end})\n{body}", path.display());
    if truncated {
        if !presentation.ends_with('\n') {
            presentation.push('\n');
        }
        presentation.push_str(&format!(
            "[truncated: showing {shown_lines} lines; pass offset={next} to continue from this path]"
        ));
    }
    LoadedSkill {
        presentation,
        truncated,
        locator: OutputLocator {
            path: Some(path.to_string_lossy().into_owned()),
            line: if truncated { Some(next) } else { Some(start) },
        },
    }
}

fn truncate_at_line_boundary(content: &str, max_bytes: usize) -> (&str, bool) {
    if content.len() <= max_bytes {
        return (content, false);
    }
    let mut end = max_bytes.min(content.len());
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    let prefix = &content[..end];
    if let Some(i) = prefix.rfind('\n') {
        (&content[..i], true)
    } else {
        (prefix, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::context::{CallIdentity, ToolOutcome};
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
    fn truncated_skill_returns_real_path_and_line() {
        let mut body = String::from("---\nname: big\ndescription: large\n---\n");
        for i in 0..2500 {
            body.push_str(&format!("line-{i:04} {}\n", "x".repeat(24)));
        }
        assert!(body.len() > MAX_TOOL_PRESENTATION_BYTES);

        let path = PathBuf::from("/tmp/codeg-agent-skills/big/SKILL.md");
        let loaded = present_skill_markdown(&path, &body, MAX_TOOL_PRESENTATION_BYTES);
        assert!(loaded.truncated);
        assert!(
            loaded
                .presentation
                .contains("/tmp/codeg-agent-skills/big/SKILL.md"),
            "{}",
            loaded.presentation
        );
        assert!(
            loaded.presentation.contains("(lines 1-"),
            "{}",
            loaded.presentation
        );
        assert!(
            loaded.presentation.contains("pass offset="),
            "{}",
            loaded.presentation
        );
        assert!(
            loaded.presentation.contains("to continue from this path"),
            "{}",
            loaded.presentation
        );
        let locator = loaded.locator;
        assert_eq!(
            locator.path.as_deref(),
            Some("/tmp/codeg-agent-skills/big/SKILL.md")
        );
        let next = locator.line.expect("next line");
        assert!(next > 1, "continuation line {next}");
        assert!(
            loaded.presentation.contains(&format!("pass offset={next}")),
            "{}",
            loaded.presentation
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
        assert!(out.contains(&md.to_string_lossy().into_owned()), "{out}");
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
    async fn skill_tool_truncated_body_keeps_real_path_and_line() {
        let launch = tempfile::tempdir().expect("cwd");
        let skills = tempfile::tempdir().expect("skills");
        let mut body = String::from("---\nname: big\ndescription: large\n---\n");
        for i in 0..2500 {
            body.push_str(&format!("line-{i:04} {}\n", "x".repeat(24)));
        }
        let md = write_skill(skills.path(), "big", &body);
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
            .call(&mut tctx, SkillArgs { name: "big".into() })
            .await
            .expect("skill");
        let path = md.to_string_lossy();
        assert!(out.contains(path.as_ref()), "{out}");
        assert!(out.contains("(lines 1-"), "{out}");
        assert!(out.contains("pass offset="), "{out}");
        assert!(out.contains("to continue from this path"), "{out}");
        let fact = ctx
            .recorder
            .store()
            .lock()
            .expect("store")
            .fact("call_s")
            .cloned()
            .expect("fact");
        assert!(fact.truncated);
        assert_eq!(
            fact.output_locator.as_ref().and_then(|l| l.path.as_deref()),
            Some(path.as_ref())
        );
        assert!(
            fact.output_locator
                .as_ref()
                .and_then(|l| l.line)
                .unwrap_or(0)
                > 1
        );
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
                .contains("skill not found: missing"),
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
