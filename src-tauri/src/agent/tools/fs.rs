//! File tools wrapping `FileSystemRuntime`.

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use sacp::schema::{ReadTextFileRequest, WriteTextFileRequest};
use serde::Deserialize;
use serde_json::json;

use super::{map_fs_error, NativeToolCtx};
use crate::agent::context::MAX_TOOL_PRESENTATION_BYTES;

const DEFAULT_READ_LIMIT: u32 = 2000;

#[derive(Clone)]
pub struct ReadFileTool {
    ctx: NativeToolCtx,
}

#[derive(Clone)]
pub struct WriteFileTool {
    ctx: NativeToolCtx,
}

#[derive(Clone)]
pub struct EditFileTool {
    ctx: NativeToolCtx,
}

impl ReadFileTool {
    pub fn new(ctx: NativeToolCtx) -> Self {
        Self { ctx }
    }
}

impl WriteFileTool {
    pub fn new(ctx: NativeToolCtx) -> Self {
        Self { ctx }
    }
}

impl EditFileTool {
    pub fn new(ctx: NativeToolCtx) -> Self {
        Self { ctx }
    }
}

#[derive(Debug, Deserialize)]
pub struct ReadFileArgs {
    pub path: String,
    #[serde(default)]
    pub offset: Option<u32>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct WriteFileArgs {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct EditFileArgs {
    pub path: String,
    pub old_string: String,
    pub new_string: String,
    #[serde(default)]
    pub replace_all: bool,
}

impl Tool for ReadFileTool {
    const NAME: &'static str = "read_file";
    type Args = ReadFileArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Read a text file. Paths may be relative to the session working directory. \
         Truncated to 2000 lines or 32KB, whichever is hit first. Use offset/limit \
         for large files; if the result says `pass offset=N`, continue from there."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path" },
                "offset": { "type": "integer", "description": "1-based starting line" },
                "limit": { "type": "integer", "description": "Maximum lines to return (default 2000)" }
            },
            "required": ["path"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let raw = json!({
            "path": args.path,
            "offset": args.offset,
            "limit": args.limit,
        });
        let fact = self.ctx.begin(Self::NAME, raw).await?;
        match read_file(&self.ctx, args).await {
            Ok(out) => self.ctx.finish_ok(fact, out).await,
            Err(err) => Err(self.ctx.finish_err(fact, err).await),
        }
    }
}

impl Tool for WriteFileTool {
    const NAME: &'static str = "write_file";
    type Args = WriteFileArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Write a text file, replacing any existing contents. Paths may be relative \
         to the session working directory."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path" },
                "content": { "type": "string", "description": "Entire file contents" }
            },
            "required": ["path", "content"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let raw = json!({ "path": args.path, "content": args.content });
        let fact = self.ctx.begin(Self::NAME, raw).await?;
        match write_file(&self.ctx, args).await {
            Ok(out) => self.ctx.finish_ok(fact, out).await,
            Err(err) => Err(self.ctx.finish_err(fact, err).await),
        }
    }
}

impl Tool for EditFileTool {
    const NAME: &'static str = "edit_file";
    type Args = EditFileArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Replace text in a file using the same filesystem runtime as write_file. \
         Fails if old_string is missing or not unique unless replace_all is true."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "old_string": { "type": "string" },
                "new_string": { "type": "string" },
                "replace_all": { "type": "boolean" }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let raw = json!({
            "path": args.path,
            "old_string": args.old_string,
            "new_string": args.new_string,
            "replace_all": args.replace_all,
        });
        let fact = self.ctx.begin(Self::NAME, raw).await?;
        match edit_file(&self.ctx, args).await {
            Ok(out) => self.ctx.finish_ok(fact, out).await,
            Err(err) => Err(self.ctx.finish_err(fact, err).await),
        }
    }
}

async fn read_file(ctx: &NativeToolCtx, args: ReadFileArgs) -> Result<String, ToolExecutionError> {
    let path = ctx.resolve_path(&args.path)?;
    if matches!(args.offset, Some(0)) {
        return Err(
            ToolExecutionError::invalid_args("offset must be >= 1 (1-based line number)")
                .with_model_feedback("offset must be >= 1"),
        );
    }
    let offset = args.offset.unwrap_or(1);
    let limit = args.limit.unwrap_or(DEFAULT_READ_LIMIT);
    let mut request = ReadTextFileRequest::new(ctx.session_id.clone(), &path).line(offset);
    request = request.limit(limit);
    let response = ctx.fs.read_text_file(request).await.map_err(map_fs_error)?;
    Ok(format_read_page(
        &path,
        offset,
        limit,
        &response.content,
        MAX_TOOL_PRESENTATION_BYTES,
    ))
}

fn format_read_page(
    path: &std::path::Path,
    offset: u32,
    limit: u32,
    content: &str,
    max_bytes: usize,
) -> String {
    let header_budget = format!("# {} (lines {offset}-999999)\n", path.display()).len();
    let footer_budget = 96;
    let body_budget = max_bytes
        .saturating_sub(header_budget)
        .saturating_sub(footer_budget)
        .max(1);
    let (body, byte_capped) = take_lines_upto_bytes(content, body_budget);
    let shown_lines = if body.is_empty() {
        0
    } else {
        body.lines().count() as u32
    };
    let end = if shown_lines == 0 {
        offset.saturating_sub(1)
    } else {
        offset.saturating_add(shown_lines.saturating_sub(1))
    };
    let mut out = format!("# {} (lines {offset}-{end})\n{body}", path.display());
    let more = byte_capped || shown_lines >= limit;
    if more && shown_lines > 0 {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&format!(
            "[truncated: showing {shown_lines} lines; pass offset={} to continue from this path]",
            offset.saturating_add(shown_lines)
        ));
    }
    out
}

fn take_lines_upto_bytes(content: &str, max_bytes: usize) -> (&str, bool) {
    if content.len() <= max_bytes {
        return (content, false);
    }
    let mut end = max_bytes.min(content.len());
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    let prefix = &content[..end];
    if let Some(i) = prefix.rfind('\n') {
        return (&content[..=i], true);
    }
    // A single line is larger than the page: keep it whole so offset still
    // advances, even if this page exceeds the byte budget.
    if let Some(i) = content.find('\n') {
        return (&content[..=i], true);
    }
    (content, false)
}

async fn write_file(
    ctx: &NativeToolCtx,
    args: WriteFileArgs,
) -> Result<String, ToolExecutionError> {
    let path = ctx.resolve_path(&args.path)?;
    let bytes = args.content.len();
    ctx.fs
        .write_text_file(WriteTextFileRequest::new(
            ctx.session_id.clone(),
            &path,
            args.content,
        ))
        .await
        .map_err(map_fs_error)?;
    Ok(format!("wrote {bytes} bytes to {}", path.display()))
}

async fn edit_file(ctx: &NativeToolCtx, args: EditFileArgs) -> Result<String, ToolExecutionError> {
    if args.old_string.is_empty() {
        return Err(
            ToolExecutionError::invalid_args("old_string must not be empty")
                .with_model_feedback("old_string must not be empty"),
        );
    }
    let path = ctx.resolve_path(&args.path)?;
    let current = ctx
        .fs
        .read_text_file(ReadTextFileRequest::new(ctx.session_id.clone(), &path))
        .await
        .map_err(map_fs_error)?;
    let matches = current.content.matches(&args.old_string).count();
    if matches == 0 {
        return Err(ToolExecutionError::not_found(format!(
            "old_string not found in {}",
            path.display()
        ))
        .with_model_feedback(format!("old_string was not found in {}", path.display())));
    }
    if matches > 1 && !args.replace_all {
        return Err(ToolExecutionError::invalid_args(format!(
            "old_string matched {matches} times in {}; set replace_all=true or provide a unique string",
            path.display()
        ))
        .with_model_feedback(format!(
            "old_string matched {matches} times; not unique. Set replace_all=true or provide a unique string."
        )));
    }
    let next = if args.replace_all {
        current.content.replace(&args.old_string, &args.new_string)
    } else {
        current
            .content
            .replacen(&args.old_string, &args.new_string, 1)
    };
    ctx.fs
        .write_text_file(WriteTextFileRequest::new(
            ctx.session_id.clone(),
            &path,
            next,
        ))
        .await
        .map_err(map_fs_error)?;
    Ok(format!(
        "updated {} ({matches} replacement{})",
        path.display(),
        if matches == 1 { "" } else { "s" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::context::{CallIdentity, ToolOutcome};
    use crate::agent::tools::test_tool_ctx;
    use rig::tool::Tool;
    use std::fs;

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    #[tokio::test]
    async fn read_write_edit_round_trip() {
        let dir = temp_dir();
        let path = dir.path().join("note.txt");
        fs::write(&path, "hello world\nsecond\n").expect("seed");

        let ctx = test_tool_ctx(dir.path(), "read_file", "call_r");
        let mut tctx = ToolContext::new();
        let read = ReadFileTool::new(ctx.clone())
            .call(
                &mut tctx,
                ReadFileArgs {
                    path: "note.txt".into(),
                    offset: Some(1),
                    limit: Some(10),
                },
            )
            .await
            .expect("read");
        assert!(read.contains("hello world"), "{read}");

        ctx.identity.set(CallIdentity {
            turn_id: 1,
            turn_key: "s:1".into(),
            tool_call_id: "call_w".into(),
            function_name: "write_file".into(),
        });
        WriteFileTool::new(ctx.clone())
            .call(
                &mut tctx,
                WriteFileArgs {
                    path: "note.txt".into(),
                    content: "alpha\nbeta\n".into(),
                },
            )
            .await
            .expect("write");
        assert_eq!(
            fs::read_to_string(&path).expect("read back"),
            "alpha\nbeta\n"
        );

        ctx.identity.set(CallIdentity {
            turn_id: 1,
            turn_key: "s:1".into(),
            tool_call_id: "call_e".into(),
            function_name: "edit_file".into(),
        });
        EditFileTool::new(ctx.clone())
            .call(
                &mut tctx,
                EditFileArgs {
                    path: "note.txt".into(),
                    old_string: "beta".into(),
                    new_string: "gamma".into(),
                    replace_all: false,
                },
            )
            .await
            .expect("edit");
        assert_eq!(fs::read_to_string(&path).expect("edited"), "alpha\ngamma\n");

        let fact = ctx
            .recorder
            .store()
            .lock()
            .expect("store")
            .fact("call_w")
            .cloned();
        assert_eq!(fact.and_then(|f| f.outcome), Some(ToolOutcome::Success));
    }

    #[tokio::test]
    async fn edit_rejects_non_unique_without_replace_all() {
        let dir = temp_dir();
        fs::write(dir.path().join("dup.txt"), "x x x\n").expect("seed");
        let ctx = test_tool_ctx(dir.path(), "edit_file", "call_e");
        let mut tctx = ToolContext::new();
        let err = EditFileTool::new(ctx)
            .call(
                &mut tctx,
                EditFileArgs {
                    path: "dup.txt".into(),
                    old_string: "x".into(),
                    new_string: "y".into(),
                    replace_all: false,
                },
            )
            .await
            .expect_err("must refuse non-unique");
        assert!(
            err.model_feedback()
                .unwrap_or_default()
                .contains("matched 3 times"),
            "{err:?}"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("dup.txt")).unwrap(),
            "x x x\n"
        );
    }

    #[tokio::test]
    async fn started_ack_failure_does_not_write() {
        let dir = temp_dir();
        let target = dir.path().join("nope.txt");
        let ctx = test_tool_ctx(dir.path(), "write_file", "call_w");
        ctx.recorder.fail_next_started();
        let mut tctx = ToolContext::new();
        let err = WriteFileTool::new(ctx)
            .call(
                &mut tctx,
                WriteFileArgs {
                    path: "nope.txt".into(),
                    content: "secret".into(),
                },
            )
            .await
            .expect_err("started ack fails closed");
        let _ = err;
        assert!(!target.exists(), "write must not run before started ack");
    }

    #[tokio::test]
    async fn write_respects_strict_policy() {
        let dir = temp_dir();
        let outside = tempfile::tempdir().expect("outside");
        let ctx = test_tool_ctx(dir.path(), "write_file", "call_w");
        let mut tctx = ToolContext::new();
        let err = WriteFileTool::new(ctx)
            .call(
                &mut tctx,
                WriteFileArgs {
                    path: outside.path().join("x.txt").to_string_lossy().into(),
                    content: "nope".into(),
                },
            )
            .await
            .expect_err("outside write");
        assert!(
            err.model_feedback()
                .unwrap_or_default()
                .contains("outside the allowed"),
            "{err:?}"
        );
        assert!(!outside.path().join("x.txt").exists());
    }

    #[tokio::test]
    async fn read_file_pages_past_the_byte_cap_with_offset() {
        let dir = temp_dir();
        let mut body = String::new();
        for i in 1..=800 {
            body.push_str(&format!("LINE-{i:04} {}\n", "x".repeat(80)));
        }
        fs::write(dir.path().join("big.txt"), &body).expect("seed");

        let ctx = test_tool_ctx(dir.path(), "read_file", "call_r1");
        let mut tctx = ToolContext::new();
        let first = ReadFileTool::new(ctx.clone())
            .call(
                &mut tctx,
                ReadFileArgs {
                    path: "big.txt".into(),
                    offset: Some(1),
                    limit: Some(2000),
                },
            )
            .await
            .expect("first page");
        assert!(first.contains("LINE-0001"), "{first}");
        assert!(
            first.contains("pass offset="),
            "byte-capped page must say how to continue: {first}"
        );
        assert!(
            !first.contains("LINE-0800"),
            "first page must not include the tail: {first}"
        );
        let next = first
            .rsplit_once("pass offset=")
            .and_then(|(_, rest)| {
                rest.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse::<u32>()
                    .ok()
            })
            .expect("offset in continuation hint");
        assert!(next > 1, "next offset {next}");

        ctx.identity.set(CallIdentity {
            turn_id: 1,
            turn_key: "s:1".into(),
            tool_call_id: "call_r2".into(),
            function_name: "read_file".into(),
        });
        let second = ReadFileTool::new(ctx)
            .call(
                &mut tctx,
                ReadFileArgs {
                    path: "big.txt".into(),
                    offset: Some(next),
                    limit: Some(2000),
                },
            )
            .await
            .expect("second page");
        assert!(
            second.contains(&format!("LINE-{next:04}")),
            "second page must start at the continuation line: {second}"
        );
        assert!(
            !second.contains("LINE-0001"),
            "second page must not repeat the first line: {second}"
        );
        assert!(
            second.contains("LINE-0800") || second.contains("pass offset="),
            "second page must reach the tail or offer another offset: {second}"
        );
    }
}
