//! 解析 Markdown 的展示元数据、正文摘要与标题目录。
//! 供笔记阅读、全文搜索和目录标题使用，不把解析结果作为生成任务或数据库状态。
//! 有效 YAML 与内部注释在阅读正文中隐藏；损坏的头部保留原文并提示，避免吞掉用户内容。

use std::collections::HashMap;

use serde::Serialize;
use serde_yaml::Value;

#[derive(Debug, Clone, Serialize)]
pub struct Heading {
    pub id: String,
    pub title: String,
    pub level: usize,
}

pub struct Document {
    pub metadata: Value,
    pub body: String,
    pub format_warning: bool,
    pub headings: Vec<Heading>,
}

impl Document {
    pub fn parse(source: &str) -> Self {
        let source = source.trim_start_matches('\u{feff}');
        let mut metadata = Value::Null;
        let mut body = source;
        let mut format_warning = false;
        if source
            .lines()
            .next()
            .is_some_and(|line| line.trim_end() == "---")
        {
            let start = source.find('\n').map_or(source.len(), |i| i + 1);
            let mut end = start;
            let mut closed = false;
            for line in source[start..].split_inclusive('\n') {
                if line.trim_end() == "---" {
                    match serde_yaml::from_str::<Value>(&source[start..end]) {
                        Ok(value @ Value::Mapping(_)) => {
                            metadata = value;
                            body = &source[end + line.len()..];
                        }
                        _ => format_warning = true,
                    }
                    closed = true;
                    break;
                }
                end += line.len();
            }
            format_warning |= !closed;
        }
        let body = strip_comments(body).trim().to_string();
        let headings = headings(&body);
        Self {
            metadata,
            body,
            format_warning,
            headings,
        }
    }

    pub fn string(&self, key: &str) -> Option<String> {
        match self.metadata.get(key)? {
            Value::String(value) => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        }
    }

    pub fn strings(&self, key: &str) -> Vec<String> {
        match self.metadata.get(key) {
            Some(Value::Sequence(values)) => values
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            Some(Value::String(value)) => vec![value.clone()],
            _ => Vec::new(),
        }
    }

    pub fn title(&self) -> Option<String> {
        self.string("title")
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                self.headings
                    .iter()
                    .find(|h| h.level == 1)
                    .map(|h| h.title.clone())
            })
    }

    pub fn summary(&self) -> String {
        self.string("summary")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                self.body
                    .lines()
                    .map(str::trim)
                    .find(|line| {
                        !line.is_empty()
                            && !line.starts_with('#')
                            && !line.starts_with("```")
                            && !line.starts_with("~~~")
                            && !line.starts_with("<!--")
                    })
                    .unwrap_or("")
                    .chars()
                    .take(220)
                    .collect()
            })
    }

    /// Catalog admission is structural, not a claim of semantic correctness.
    /// Index pages, metadata-only wrappers and link-only shells aren't notes.
    pub fn has_content(&self) -> bool {
        self.body.lines().any(|line| {
            let text = line.trim().trim_start_matches(['-', '*', '+', '>']).trim();
            !text.is_empty()
                && !text.starts_with('#')
                && !matches!(text, "暂无" | "暂无。" | "None" | "None." | "---" | "```")
                && !(text.starts_with("[[") && text.ends_with("]]"))
                && !(text.starts_with('[') && text.ends_with(')') && text.contains("]("))
                && !text.starts_with("<!--")
                && !text.starts_with("```")
                && !text.starts_with("~~~")
        })
    }
}

fn fence(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start();
    let marker = trimmed.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let count = trimmed.chars().take_while(|c| *c == marker).count();
    (count >= 3).then_some((marker, count))
}

fn strip_comments(source: &str) -> String {
    let mut out = String::new();
    let mut fenced: Option<(char, usize)> = None;
    let mut comment = false;
    for line in source.split_inclusive('\n') {
        if !comment {
            if let Some((marker, count)) = fence(line) {
                if let Some((open, length)) = fenced {
                    if marker == open && count >= length {
                        fenced = None;
                    }
                } else {
                    fenced = Some((marker, count));
                }
                out.push_str(line);
                continue;
            }
            if fenced.is_some() || line.starts_with("    ") || line.starts_with('\t') {
                out.push_str(line);
                continue;
            }
        }
        let mut tail = line;
        while !tail.is_empty() {
            if comment {
                if let Some(end) = tail.find("-->") {
                    tail = &tail[end + 3..];
                    comment = false;
                } else {
                    break;
                }
            } else if let Some(start) = tail.find("<!--") {
                // A comment token inside an inline code span is literal text.
                let prefix = &tail[..start];
                if prefix.chars().filter(|c| *c == '`').count() % 2 == 1 {
                    out.push_str(tail);
                    break;
                }
                out.push_str(prefix);
                tail = &tail[start + 4..];
                comment = true;
            } else {
                out.push_str(tail);
                break;
            }
        }
    }
    out
}

pub fn heading_id(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                Some(c)
            } else if c.is_whitespace() {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}

fn headings(body: &str) -> Vec<Heading> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut result = Vec::new();
    let mut fenced: Option<(char, usize)> = None;
    for line in body.lines() {
        if let Some((marker, count)) = fence(line) {
            if let Some((open, length)) = fenced {
                if marker == open && count >= length {
                    fenced = None;
                }
            } else {
                fenced = Some((marker, count));
            }
            continue;
        }
        if fenced.is_some() {
            continue;
        }
        let level = line.chars().take_while(|c| *c == '#').count();
        if !(1..=6).contains(&level) || !line[level..].starts_with(' ') {
            continue;
        }
        let title = line[level..]
            .trim()
            .trim_end_matches('#')
            .trim()
            .to_string();
        let base = heading_id(&title);
        let count = seen.entry(base.clone()).or_default();
        let id = if *count == 0 {
            base
        } else {
            format!("{base}-{count}")
        };
        *count += 1;
        result.push(Heading { id, title, level });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_valid_metadata_and_control_comments_but_keeps_human_body_and_code() {
        let doc = Document::parse("---\ntitle: 易读笔记\ntype: method\n---\n<!-- codeg-content:start -->\n# 方法\n\n实际步骤。\n<!-- codeg-content:end -->\n<!-- user:start -->\n我的补充。\n<!-- user:end -->\n```md\n<!-- literal -->\n[[literal]]\n```\n");
        assert_eq!(doc.title().as_deref(), Some("易读笔记"));
        assert!(!doc.body.contains("type: method"));
        assert!(!doc.body.contains("codeg-content"));
        assert!(doc.body.contains("我的补充。"));
        assert!(doc.body.contains("<!-- literal -->"));
        assert_eq!(doc.headings.len(), 1);
        assert!(doc.has_content());
    }

    #[test]
    fn malformed_yaml_is_not_erased() {
        for source in [
            "---\ntitle: [invalid\n---\n正文",
            "---\ntitle: 文本\n没有结束标记",
        ] {
            let doc = Document::parse(source);
            assert!(doc.format_warning);
            assert!(doc.body.contains("title:"));
        }
    }

    #[test]
    fn shell_pages_are_not_counted_as_notes() {
        for body in [
            "# 项目\n\n暂无。",
            "# 目录\n- [[method/one]]",
            "---\ntitle: 空\n---\n# 空",
        ] {
            assert!(!Document::parse(body).has_content(), "{body}");
        }
        assert!(Document::parse("# 决定\n保留旧协议直到所有调用方升级。").has_content());
    }

    #[test]
    fn repeated_chinese_headings_have_stable_distinct_anchors() {
        let doc = Document::parse("# 结论\n## 检查步骤\n## 检查步骤\n```md\n# 示例\n```");
        assert_eq!(
            doc.headings
                .iter()
                .map(|h| h.id.as_str())
                .collect::<Vec<_>>(),
            ["结论", "检查步骤", "检查步骤-1"]
        );
    }
}
