//! 在会话与导入元信息落入 Wiki 前遮蔽常见凭据和敏感字面值。
//! source 和 import 复用同一脱敏规则，并把是否脱敏记入来源元数据。
//! 该处理不请求外部服务，也不把脱敏后的文字当作已经验证的业务事实。

use regex::Regex;
use std::sync::OnceLock;

fn patterns() -> &'static [(Regex, &'static str)] {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            (
                Regex::new(r"(?i)(authorization)\s*[:=]\s*[^\r\n]+").expect("authorization regex"),
                "$1: [REDACTED]",
            ),
            (
                Regex::new(r"(?i)((?:set-)?cookie)\s*[:=]\s*[^\r\n]+").expect("cookie regex"),
                "$1: [REDACTED]",
            ),
            (
                Regex::new(r"(?i)bearer\s+[A-Za-z0-9._\-+=/]+").expect("bearer regex"),
                "Bearer [REDACTED]",
            ),
            (
                Regex::new(r"(https?://)[^/\s:@]+:[^/\s:@]+@").expect("url credential regex"),
                "$1[REDACTED]@",
            ),
            (
                Regex::new(
                    r"(?i)\b([A-Za-z_]*(?:SECRET|TOKEN|PASSWORD|PASSWD|API[_-]?KEY|PRIVATE[_-]?KEY|ACCESS[_-]?KEY)[A-Za-z0-9_]*)\s*=\s*\S+",
                )
                .expect("env key regex"),
                "$1=[REDACTED]",
            ),
        ]
    })
}

/// Replace known secret patterns. Returns the redacted string and whether any
/// substitution ran. The original is not logged.
pub fn redact_text(input: &str) -> (String, bool) {
    let mut out = input.to_string();
    let mut changed = false;
    for (re, replacement) in patterns() {
        let next = re.replace_all(&out, *replacement).into_owned();
        if next != out {
            changed = true;
            out = next;
        }
    }
    (out, changed)
}

/// Redact a snapshot's user/assistant/tool summary fields in place.
pub fn redact_snapshot(snap: &mut crate::wiki::snapshot::WikiTurnSnapshot) -> bool {
    let mut any = false;
    let (user, u) = redact_text(&snap.user_text);
    snap.user_text = user;
    any |= u;
    let (asst, a) = redact_text(&snap.assistant_text);
    snap.assistant_text = asst;
    any |= a;
    for obs in &mut snap.tool_observations {
        let (summary, s) = redact_text(&obs.summary);
        obs.summary = summary;
        any |= s;
        if let Some(path) = obs.path.take() {
            let (p, c) = redact_text(&path);
            obs.path = Some(p);
            any |= c;
        }
    }
    for change in &mut snap.file_changes {
        let (p, c) = redact_text(&change.path);
        change.path = p;
        any |= c;
    }
    any
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_authorization_cookie_bearer_env_and_url() {
        let src = concat!(
            "Authorization: Bearer supersecret\n",
            "Cookie: session=abc; token=xyz\n",
            "Authorization: Basic dXNlcjpwYXNz\n",
            "API_KEY=sk-live-123\n",
            "DB_PASSWORD=hunter2\n",
            "clone https://user:p4ss@github.com/org/repo.git\n",
        );
        let (out, changed) = redact_text(src);
        assert!(changed);
        assert!(!out.contains("supersecret"), "bearer must not leak");
        assert!(!out.contains("session=abc"));
        assert!(!out.contains("dXNlcjpwYXNz"));
        assert!(!out.contains("sk-live-123"));
        assert!(!out.contains("hunter2"));
        assert!(!out.contains("user:p4ss"));
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn leaves_ordinary_text() {
        let (out, changed) = redact_text("fixed the login form");
        assert!(!changed);
        assert_eq!(out, "fixed the login form");
    }
}
