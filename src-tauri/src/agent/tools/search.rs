//! Glob/grep over the workspace using the same canonical read gate as fs tools.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;
use regex::Regex;
use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Deserialize;
use serde_json::json;

use super::{map_fs_error, NativeToolCtx};

const GLOB_MAX_RESULTS: usize = 200;
const GREP_DEFAULT_RESULTS: usize = 100;
const GREP_MAX_RESULTS: usize = 1000;
const GREP_MAX_LINE_BYTES: usize = 8 * 1024;
const GREP_SKIP_FILE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone)]
pub struct GlobTool {
    ctx: NativeToolCtx,
}

#[derive(Clone)]
pub struct GrepTool {
    ctx: NativeToolCtx,
}

impl GlobTool {
    pub fn new(ctx: NativeToolCtx) -> Self {
        Self { ctx }
    }
}

impl GrepTool {
    pub fn new(ctx: NativeToolCtx) -> Self {
        Self { ctx }
    }
}

#[derive(Debug, Deserialize)]
pub struct GlobArgs {
    pub pattern: String,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GrepArgs {
    pub pattern: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub glob: Option<String>,
    #[serde(default)]
    pub max_results: Option<u32>,
}

impl Tool for GlobTool {
    const NAME: &'static str = "glob";
    type Args = GlobArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Find files matching a glob pattern. Walks gitignored trees and re-checks \
         every path (including symlink targets) with the same read policy as read_file."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Glob pattern, e.g. **/*.rs" },
                "path": { "type": "string", "description": "Search root; defaults to the session cwd" }
            },
            "required": ["pattern"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let raw = json!({ "pattern": args.pattern, "path": args.path });
        let fact = self.ctx.begin(Self::NAME, raw).await?;
        match glob_files(&self.ctx, args).await {
            Ok(out) => self.ctx.finish_ok(fact, out).await,
            Err(err) => Err(self.ctx.finish_err(fact, err).await),
        }
    }
}

impl Tool for GrepTool {
    const NAME: &'static str = "grep";
    type Args = GrepArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Search file contents with a regex. Each opened file is gated by the same \
         canonical read check as read_file (symlink targets included). Truncated \
         to 100 matches by default; if truncated, raise max_results or refine the \
         path/pattern."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Rust regex" },
                "path": { "type": "string", "description": "File or directory to search" },
                "glob": { "type": "string", "description": "Optional filename glob filter" },
                "max_results": { "type": "integer", "description": "Maximum matches (default 100). If truncated, double this value for more." }
            },
            "required": ["pattern"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let raw = json!({
            "pattern": args.pattern,
            "path": args.path,
            "glob": args.glob,
            "max_results": args.max_results,
        });
        let fact = self.ctx.begin(Self::NAME, raw).await?;
        match grep_files(&self.ctx, args).await {
            Ok(out) => self.ctx.finish_ok(fact, out).await,
            Err(err) => Err(self.ctx.finish_err(fact, err).await),
        }
    }
}

async fn glob_files(ctx: &NativeToolCtx, args: GlobArgs) -> Result<String, ToolExecutionError> {
    if args.pattern.trim().is_empty() {
        return Err(
            ToolExecutionError::invalid_args("pattern must not be empty")
                .with_model_feedback("glob pattern must not be empty"),
        );
    }
    let root = search_root(ctx, args.path.as_deref())?;
    gate_root(ctx, &root)?;
    let pattern = args.pattern.clone();
    let fs = ctx.fs.clone();
    let matches = tokio::task::spawn_blocking(move || collect_glob_matches(&fs, &root, &pattern))
        .await
        .map_err(|err| {
            ToolExecutionError::other(format!("glob walk failed: {err}"))
                .with_model_feedback("glob walk failed")
        })??;
    format_glob_results(matches)
}

async fn grep_files(ctx: &NativeToolCtx, args: GrepArgs) -> Result<String, ToolExecutionError> {
    if args.pattern.trim().is_empty() {
        return Err(
            ToolExecutionError::invalid_args("pattern must not be empty")
                .with_model_feedback("grep pattern must not be empty"),
        );
    }
    let regex = Regex::new(&args.pattern).map_err(|err| {
        ToolExecutionError::invalid_args(format!("invalid regex: {err}"))
            .with_model_feedback(format!("invalid regex: {err}"))
    })?;
    let root = search_root(ctx, args.path.as_deref())?;
    gate_root(ctx, &root)?;
    let max = args
        .max_results
        .map(|n| n as usize)
        .unwrap_or(GREP_DEFAULT_RESULTS)
        .clamp(1, GREP_MAX_RESULTS);
    let glob = args.glob.clone();
    let fs = ctx.fs.clone();
    let hits = tokio::task::spawn_blocking(move || {
        collect_grep_matches(&fs, &root, &regex, glob.as_deref(), max)
    })
    .await
    .map_err(|err| {
        ToolExecutionError::other(format!("grep walk failed: {err}"))
            .with_model_feedback("grep walk failed")
    })??;
    format_grep_results(hits, max)
}

fn search_root(ctx: &NativeToolCtx, path: Option<&str>) -> Result<PathBuf, ToolExecutionError> {
    match path {
        Some(path) => ctx.resolve_path(path),
        None => Ok(ctx.launch_cwd.clone()),
    }
}

fn gate_root(ctx: &NativeToolCtx, root: &Path) -> Result<(), ToolExecutionError> {
    ctx.fs.check_read(root).map_err(map_fs_error)
}

fn compile_glob(
    root: &Path,
    pattern: &str,
) -> Result<ignore::overrides::Override, ToolExecutionError> {
    let mut builder = OverrideBuilder::new(root);
    builder.add(pattern).map_err(|err| {
        ToolExecutionError::invalid_args(format!("invalid glob pattern `{pattern}`: {err}"))
            .with_model_feedback(format!("invalid glob pattern: {pattern}"))
    })?;
    // `*.rs` should match in subdirectories, like other coding-agent glob tools.
    if !pattern.contains('/') && !pattern.contains('\\') && !pattern.starts_with("**/") {
        let _ = builder.add(&format!("**/{pattern}"));
    }
    builder.build().map_err(|err| {
        ToolExecutionError::invalid_args(format!("invalid glob pattern `{pattern}`: {err}"))
            .with_model_feedback(format!("invalid glob pattern: {pattern}"))
    })
}

fn walk_files(root: &Path) -> ignore::Walk {
    WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(false)
        .ignore(true)
        .parents(true)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build()
}

struct GlobHit {
    path: PathBuf,
    mtime: Option<SystemTime>,
}

fn collect_glob_matches(
    fs: &crate::acp::file_system_runtime::FileSystemRuntime,
    root: &Path,
    pattern: &str,
) -> Result<Vec<GlobHit>, ToolExecutionError> {
    if root.is_file() {
        if !glob_matches(root, pattern, root, false)? {
            return Ok(Vec::new());
        }
        if fs.check_read(root).is_err() {
            return Ok(Vec::new());
        }
        return Ok(vec![GlobHit {
            path: root.to_path_buf(),
            mtime: std::fs::metadata(root).ok().and_then(|m| m.modified().ok()),
        }]);
    }

    let matcher = compile_glob(root, pattern)?;
    let mut hits = Vec::new();
    for entry in walk_files(root) {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path == root {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            continue;
        }
        if !matcher.matched(path, false).is_whitelist() {
            continue;
        }
        if fs.check_read(path).is_err() {
            continue;
        }
        hits.push(GlobHit {
            path: path.to_path_buf(),
            mtime: entry.metadata().ok().and_then(|m| m.modified().ok()),
        });
    }
    hits.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.path.cmp(&b.path)));
    Ok(hits)
}

fn glob_matches(
    root: &Path,
    pattern: &str,
    path: &Path,
    is_dir: bool,
) -> Result<bool, ToolExecutionError> {
    Ok(compile_glob(root, pattern)?
        .matched(path, is_dir)
        .is_whitelist())
}

fn format_glob_results(mut hits: Vec<GlobHit>) -> Result<String, ToolExecutionError> {
    let total = hits.len();
    hits.truncate(GLOB_MAX_RESULTS);
    if hits.is_empty() {
        return Ok("0 files".to_string());
    }
    let mut out = String::new();
    if total > GLOB_MAX_RESULTS {
        out.push_str(&format!(
            "# showing {GLOB_MAX_RESULTS} of {total} files (mtime descending)\n"
        ));
    } else {
        out.push_str(&format!("# {total} files (mtime descending)\n"));
    }
    for hit in hits {
        out.push_str(&hit.path.display().to_string());
        out.push('\n');
    }
    Ok(out)
}

struct GrepHit {
    path: PathBuf,
    line: usize,
    text: String,
}

fn collect_grep_matches(
    fs: &crate::acp::file_system_runtime::FileSystemRuntime,
    root: &Path,
    regex: &Regex,
    glob: Option<&str>,
    max: usize,
) -> Result<Vec<GrepHit>, ToolExecutionError> {
    let glob_matcher = match glob {
        Some(pattern) if !pattern.is_empty() => Some(compile_glob(root, pattern)?),
        _ => None,
    };
    let mut hits = Vec::new();
    if root.is_file() {
        if let Some(matcher) = &glob_matcher {
            if !matcher.matched(root, false).is_whitelist() {
                return Ok(hits);
            }
        }
        grep_one_file(fs, root, regex, max, &mut hits);
        return Ok(hits);
    }
    for entry in walk_files(root) {
        if hits.len() >= max {
            break;
        }
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path == root {
            continue;
        }
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        if let Some(matcher) = &glob_matcher {
            if !matcher.matched(path, false).is_whitelist() {
                continue;
            }
        }
        grep_one_file(fs, path, regex, max, &mut hits);
    }
    Ok(hits)
}

fn grep_one_file(
    fs: &crate::acp::file_system_runtime::FileSystemRuntime,
    path: &Path,
    regex: &Regex,
    max: usize,
    hits: &mut Vec<GrepHit>,
) {
    if hits.len() >= max {
        return;
    }
    if fs.check_read(path).is_err() {
        return;
    }
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    if meta.len() > GREP_SKIP_FILE_BYTES {
        return;
    }
    let Ok(file) = File::open(path) else {
        return;
    };
    let mut reader = BufReader::new(file);
    let mut buf = String::new();
    let mut line_no = 0usize;
    loop {
        buf.clear();
        let Ok(n) = reader.read_line(&mut buf) else {
            return;
        };
        if n == 0 {
            break;
        }
        line_no += 1;
        if buf.contains('\0') {
            return;
        }
        if regex.is_match(&buf) {
            let mut text = buf.trim_end_matches(['\n', '\r']).to_string();
            if text.len() > GREP_MAX_LINE_BYTES {
                text.truncate(GREP_MAX_LINE_BYTES);
                text.push('…');
            }
            hits.push(GrepHit {
                path: path.to_path_buf(),
                line: line_no,
                text,
            });
            if hits.len() >= max {
                return;
            }
        }
        if buf.len() > 64 * 1024 {
            return;
        }
    }
}

fn format_grep_results(hits: Vec<GrepHit>, max: usize) -> Result<String, ToolExecutionError> {
    if hits.is_empty() {
        return Ok("0 matches".to_string());
    }
    let truncated = hits.len() >= max;
    let mut out = if truncated {
        format!(
            "# showing {max} matches ({max} matches limit reached. Use max_results={} for more, or refine path/pattern)\n",
            max.saturating_mul(2).min(GREP_MAX_RESULTS)
        )
    } else {
        format!("# {} matches\n", hits.len())
    };
    for hit in hits {
        out.push_str(&format!(
            "{}:{}:{}\n",
            hit.path.display(),
            hit.line,
            hit.text
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::context::CallIdentity;
    use crate::agent::tools::test_tool_ctx;
    use rig::tool::Tool;
    use std::fs;

    #[tokio::test]
    async fn glob_and_grep_find_workspace_files() {
        let dir = tempfile::tempdir().expect("dir");
        fs::write(dir.path().join("a.rs"), "fn alpha() {}\n").unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("src/b.rs"),
            "fn beta() { let needle = 1; }\n",
        )
        .unwrap();
        fs::write(dir.path().join("c.txt"), "needle in text\n").unwrap();

        let ctx = test_tool_ctx(dir.path(), "glob", "call_g");
        let mut tctx = ToolContext::new();
        let glob = GlobTool::new(ctx.clone())
            .call(
                &mut tctx,
                GlobArgs {
                    pattern: "*.rs".into(),
                    path: None,
                },
            )
            .await
            .expect("glob");
        assert!(glob.contains("a.rs"), "{glob}");
        assert!(glob.contains("b.rs"), "{glob}");
        assert!(!glob.contains("c.txt"), "{glob}");

        ctx.identity.set(CallIdentity {
            turn_id: 1,
            turn_key: "s:1".into(),
            tool_call_id: "call_grep".into(),
            function_name: "grep".into(),
        });
        let grep = GrepTool::new(ctx)
            .call(
                &mut tctx,
                GrepArgs {
                    pattern: "needle".into(),
                    path: None,
                    glob: Some("*.txt".into()),
                    max_results: None,
                },
            )
            .await
            .expect("grep");
        assert!(grep.contains("c.txt"), "{grep}");
        assert!(!grep.contains("b.rs"), "{grep}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn glob_and_grep_skip_symlink_escape_under_strict() {
        let dir = tempfile::tempdir().expect("dir");
        let outside = tempfile::tempdir().expect("outside");
        fs::write(outside.path().join("secret.txt"), "TOP SECRET needle\n").unwrap();
        fs::write(dir.path().join("ok.txt"), "needle inside\n").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            dir.path().join("escape.txt"),
        )
        .expect("symlink");

        let ctx = test_tool_ctx(dir.path(), "glob", "call_g");
        let mut tctx = ToolContext::new();
        let glob = GlobTool::new(ctx.clone())
            .call(
                &mut tctx,
                GlobArgs {
                    pattern: "*.txt".into(),
                    path: None,
                },
            )
            .await
            .expect("glob");
        assert!(glob.contains("ok.txt"), "{glob}");
        assert!(
            !glob.contains("escape.txt") && !glob.contains("secret.txt"),
            "symlink escape must not be listed: {glob}"
        );

        ctx.identity.set(CallIdentity {
            turn_id: 1,
            turn_key: "s:1".into(),
            tool_call_id: "call_grep".into(),
            function_name: "grep".into(),
        });
        let grep = GrepTool::new(ctx)
            .call(
                &mut tctx,
                GrepArgs {
                    pattern: "needle".into(),
                    path: None,
                    glob: None,
                    max_results: None,
                },
            )
            .await
            .expect("grep");
        assert!(grep.contains("ok.txt"), "{grep}");
        assert!(
            !grep.contains("TOP SECRET") && !grep.contains("escape.txt"),
            "grep must not follow the symlink: {grep}"
        );
    }

    #[tokio::test]
    async fn grep_raises_max_results_to_see_more() {
        let dir = tempfile::tempdir().expect("dir");
        let mut body = String::new();
        for i in 1..=20 {
            body.push_str(&format!("hit-{i:02} needle\n"));
        }
        fs::write(dir.path().join("hits.txt"), body).unwrap();

        let ctx = test_tool_ctx(dir.path(), "grep", "call_g1");
        let mut tctx = ToolContext::new();
        let first = GrepTool::new(ctx.clone())
            .call(
                &mut tctx,
                GrepArgs {
                    pattern: "needle".into(),
                    path: None,
                    glob: None,
                    max_results: Some(6),
                },
            )
            .await
            .expect("capped");
        assert!(first.contains("hit-01"), "{first}");
        assert!(first.contains("hit-06"), "{first}");
        assert!(!first.contains("hit-07"), "{first}");
        assert!(
            first.contains("max_results=12"),
            "Pi-style: raise limit, do not page with offset: {first}"
        );
        assert!(!first.contains("pass offset="), "{first}");

        ctx.identity.set(CallIdentity {
            turn_id: 1,
            turn_key: "s:1".into(),
            tool_call_id: "call_g2".into(),
            function_name: "grep".into(),
        });
        let second = GrepTool::new(ctx)
            .call(
                &mut tctx,
                GrepArgs {
                    pattern: "needle".into(),
                    path: None,
                    glob: None,
                    max_results: Some(12),
                },
            )
            .await
            .expect("raised limit");
        assert!(second.contains("hit-01"), "{second}");
        assert!(second.contains("hit-12"), "{second}");
        assert!(!second.contains("hit-13"), "{second}");
        assert!(second.contains("max_results=24"), "{second}");
    }
}
