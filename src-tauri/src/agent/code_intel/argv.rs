use std::path::{Path, PathBuf};

use super::SERENA_UVX_FROM;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedProcess {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
}

pub fn serena_mcp_args(context: &str, workspace: &Path, modes: &[String]) -> Vec<String> {
    let mut args = vec![
        "start-mcp-server".to_string(),
        "--context".to_string(),
        context.to_string(),
        "--project".to_string(),
        workspace.display().to_string(),
    ];
    for mode in modes {
        args.push("--mode".to_string());
        args.push(mode.clone());
    }
    args.push("--enable-web-dashboard".to_string());
    args.push("false".to_string());
    args.push("--open-web-dashboard".to_string());
    args.push("false".to_string());
    args
}

pub fn plan_serena_binary(
    program: PathBuf,
    context: &str,
    workspace: &Path,
    modes: &[String],
) -> PlannedProcess {
    PlannedProcess {
        program,
        args: serena_mcp_args(context, workspace, modes),
        cwd: workspace.to_path_buf(),
        env: Vec::new(),
    }
}

pub fn plan_serena_uvx(
    uvx: PathBuf,
    context: &str,
    workspace: &Path,
    modes: &[String],
) -> PlannedProcess {
    let mut args = vec![
        "--from".to_string(),
        SERENA_UVX_FROM.to_string(),
        "serena".to_string(),
    ];
    args.extend(serena_mcp_args(context, workspace, modes));
    PlannedProcess {
        program: uvx,
        args,
        cwd: workspace.to_path_buf(),
        env: Vec::new(),
    }
}

pub fn codegraph_mcp_args() -> Vec<String> {
    vec!["serve".to_string(), "--mcp".to_string()]
}

pub fn codegraph_env() -> Vec<(String, String)> {
    vec![
        ("CODEGRAPH_TELEMETRY".to_string(), "0".to_string()),
        ("DO_NOT_TRACK".to_string(), "1".to_string()),
        ("CODEGRAPH_NO_UPDATE_CHECK".to_string(), "1".to_string()),
    ]
}

pub fn plan_codegraph(program: PathBuf, workspace: &Path) -> PlannedProcess {
    PlannedProcess {
        program,
        args: codegraph_mcp_args(),
        cwd: workspace.to_path_buf(),
        env: codegraph_env(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modes() -> Vec<String> {
        vec![
            "interactive".to_string(),
            "editing".to_string(),
            "planning".to_string(),
        ]
    }

    #[test]
    fn serena_binary_args_keep_spaced_project_as_one_element() {
        let workspace = PathBuf::from("/tmp/my project");
        let program = PathBuf::from("/opt/My Tools/serena");
        let plan = plan_serena_binary(program.clone(), "codex", &workspace, &modes());
        assert_eq!(plan.program, program);
        assert_eq!(plan.cwd, workspace);
        let project = plan
            .args
            .iter()
            .position(|arg| arg == "--project")
            .expect("project flag");
        assert_eq!(plan.args[project + 1], "/tmp/my project");
        assert_eq!(plan.args.iter().filter(|arg| arg.contains(' ')).count(), 1);
        assert_eq!(plan.args.iter().filter(|arg| *arg == "--mode").count(), 3);
        assert!(plan
            .args
            .windows(2)
            .any(|pair| { pair[0] == "--enable-web-dashboard" && pair[1] == "false" }));
        assert!(plan
            .args
            .windows(2)
            .any(|pair| pair[0] == "--open-web-dashboard" && pair[1] == "false"));
        assert!(!plan.args.iter().any(|arg| arg.contains("rust-analyzer")));
        assert!(plan.env.is_empty());
    }

    #[test]
    fn serena_uvx_args_pin_v1_7_0() {
        let workspace = PathBuf::from("/work/demo");
        let plan = plan_serena_uvx(PathBuf::from("/usr/bin/uvx"), "codex", &workspace, &modes());
        assert_eq!(plan.program, PathBuf::from("/usr/bin/uvx"));
        assert_eq!(
            &plan.args[..3],
            [
                "--from".to_string(),
                "git+https://github.com/oraios/serena@v1.7.0".to_string(),
                "serena".to_string(),
            ]
        );
        assert!(plan.args.iter().any(|arg| arg == "start-mcp-server"));
        assert!(!plan
            .args
            .iter()
            .any(|arg| arg == "git+https://github.com/oraios/serena"));
        assert!(!plan.args.iter().any(|arg| arg.contains("setup")));
    }

    #[test]
    fn codegraph_serve_args_and_env_keep_spaced_program() {
        let program = PathBuf::from("/opt/My Tools/codegraph");
        let workspace = PathBuf::from("/tmp/ws");
        let plan = plan_codegraph(program.clone(), &workspace);
        assert_eq!(plan.program, program);
        assert_eq!(plan.args, vec!["serve".to_string(), "--mcp".to_string()]);
        assert_eq!(plan.cwd, workspace);
        assert!(plan
            .env
            .iter()
            .any(|(key, value)| key == "CODEGRAPH_TELEMETRY" && value == "0"));
        assert!(plan
            .env
            .iter()
            .any(|(key, value)| key == "DO_NOT_TRACK" && value == "1"));
        assert!(plan
            .env
            .iter()
            .any(|(key, value)| key == "CODEGRAPH_NO_UPDATE_CHECK" && value == "1"));
    }

    #[test]
    fn different_workspaces_do_not_share_serena_project() {
        let left = PathBuf::from("/work/left project");
        let right = PathBuf::from("/work/right");
        let left_args = serena_mcp_args("codex", &left, &modes());
        let right_args = serena_mcp_args("codex", &right, &modes());
        assert!(left_args.iter().any(|arg| arg == "/work/left project"));
        assert!(!left_args.iter().any(|arg| arg == "/work/right"));
        assert!(right_args.iter().any(|arg| arg == "/work/right"));
        assert!(!right_args.iter().any(|arg| arg.contains("left")));
    }
}
