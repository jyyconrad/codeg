use std::collections::HashSet;
use std::sync::Arc;

use rmcp::model::{JsonObject, Tool, ToolAnnotations};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolProvider {
    Serena,
    Codegraph,
}

const SERENA_DENY: &[&str] = &[
    "search_for_pattern",
    "list_dir",
    "find_file",
    "read_file",
    "create_text_file",
    "replace_content",
    "delete_lines",
    "execute_shell_command",
    "switch_modes",
];

const CODEGRAPH_DENY: &[&str] = &[
    "install",
    "uninstall",
    "ui",
    "web",
    "daemon",
    "upgrade",
    "telemetry",
    "uninit",
    "init",
    "sync",
    "codegraph_install",
    "codegraph_uninstall",
    "codegraph_init",
    "codegraph_sync",
];

pub fn tool_denied(provider: ToolProvider, name: &str) -> bool {
    match provider {
        ToolProvider::Serena => SERENA_DENY.contains(&name),
        ToolProvider::Codegraph => CODEGRAPH_DENY.contains(&name),
    }
}

/// Keep official tools, including schemas and `readOnlyHint`. Drop exact deny-list
/// names for the provider that advertised them. The first provider to offer a
/// name wins; later duplicates are skipped.
pub fn merge_provider_tools(groups: Vec<(ToolProvider, Vec<Tool>)>) -> Vec<Tool> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (provider, tools) in groups {
        for tool in tools {
            if tool_denied(provider, tool.name.as_ref()) {
                continue;
            }
            if !seen.insert(tool.name.to_string()) {
                continue;
            }
            out.push(tool);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, read_only: Option<bool>) -> Tool {
        let mut tool = Tool::new(
            name.to_string(),
            format!("{name} desc"),
            Arc::new(JsonObject::new()),
        );
        if let Some(flag) = read_only {
            tool.annotations = Some(ToolAnnotations::from_raw(
                Some(name.to_string()),
                Some(flag),
                Some(!flag),
                None,
                None,
            ));
        }
        tool
    }

    #[test]
    fn deny_list_is_exact_and_annotations_pass_through() {
        let explore = tool("codegraph_explore", Some(true));
        let init = tool("init", None);
        let find_symbol = tool("find_symbol", Some(true));
        let write = tool("replace_symbol_body", Some(false));
        let read_file = tool("read_file", Some(true));
        let read_file_extra = tool("read_file_extra", Some(true));
        let merged = merge_provider_tools(vec![
            (
                ToolProvider::Codegraph,
                vec![
                    explore,
                    init,
                    tool("codegraph_init", None),
                    tool("sync", None),
                ],
            ),
            (
                ToolProvider::Serena,
                vec![
                    find_symbol,
                    write,
                    read_file,
                    read_file_extra,
                    tool("switch_modes", None),
                    tool("search_for_pattern", None),
                    tool("list_dir", None),
                    tool("find_file", None),
                    tool("create_text_file", None),
                    tool("replace_content", None),
                    tool("delete_lines", None),
                    tool("execute_shell_command", None),
                ],
            ),
        ]);
        let names: Vec<_> = merged.iter().map(|tool| tool.name.to_string()).collect();
        assert_eq!(
            names,
            vec![
                "codegraph_explore",
                "find_symbol",
                "replace_symbol_body",
                "read_file_extra",
            ]
        );
        assert_eq!(
            merged[0]
                .annotations
                .as_ref()
                .and_then(|ann| ann.read_only_hint),
            Some(true)
        );
        assert_eq!(
            merged[2]
                .annotations
                .as_ref()
                .and_then(|ann| ann.read_only_hint),
            Some(false)
        );
        assert_eq!(
            merged[1]
                .annotations
                .as_ref()
                .and_then(|ann| ann.title.clone()),
            Some("find_symbol".to_string())
        );
        assert!(merged[3].input_schema.is_empty() || merged[3].description.is_some());
    }

    #[test]
    fn failed_provider_list_does_not_drop_the_other() {
        let explore = tool("codegraph_explore", Some(true));
        let merged = merge_provider_tools(vec![
            (ToolProvider::Codegraph, vec![explore]),
            (ToolProvider::Serena, Vec::new()),
        ]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].name.as_ref(), "codegraph_explore");
    }

    #[test]
    fn duplicate_official_name_keeps_the_first_provider() {
        let first = tool("find_symbol", Some(true));
        let second = tool("find_symbol", Some(false));
        let merged = merge_provider_tools(vec![
            (ToolProvider::Codegraph, vec![first]),
            (ToolProvider::Serena, vec![second]),
        ]);
        assert_eq!(merged.len(), 1);
        assert_eq!(
            merged[0]
                .annotations
                .as_ref()
                .and_then(|ann| ann.read_only_hint),
            Some(true)
        );
    }
}
