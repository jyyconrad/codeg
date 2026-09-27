use std::collections::HashSet;

/// No official LSP-MCP package is registered. Preset binaries are not commands.
pub fn official_lsp_provider_ids() -> &'static [&'static str] {
    &[]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspStartInput {
    pub master_enabled: bool,
    pub lsp_enabled: bool,
    pub checked: Vec<String>,
    pub detected: Vec<String>,
    pub official: Vec<String>,
    pub running: Vec<String>,
    pub max_concurrent: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LspStartPlan {
    pub start: Vec<String>,
    pub waiting: Vec<String>,
}

/// `start` is checked ∩ detected ∩ official, capped by `max_concurrent`.
/// Languages already running stay in `start` and are not evicted to make room.
/// Overflow is `waiting`. An empty official registry yields an empty plan.
pub fn plan_lsp_starts(input: &LspStartInput) -> LspStartPlan {
    if !input.master_enabled || !input.lsp_enabled {
        return LspStartPlan::default();
    }
    let detected: HashSet<&str> = input.detected.iter().map(String::as_str).collect();
    let official: HashSet<&str> = input.official.iter().map(String::as_str).collect();
    let running: HashSet<&str> = input.running.iter().map(String::as_str).collect();
    let mut desired = Vec::new();
    for key in &input.checked {
        if detected.contains(key.as_str())
            && official.contains(key.as_str())
            && !desired.iter().any(|existing: &String| existing == key)
        {
            desired.push(key.clone());
        }
    }
    let cap = input.max_concurrent as usize;
    let running_count = desired
        .iter()
        .filter(|key| running.contains(key.as_str()))
        .count();
    let mut fresh = 0usize;
    let mut start = Vec::new();
    let mut waiting = Vec::new();
    for key in desired {
        if running.contains(key.as_str()) {
            start.push(key);
        } else if running_count.saturating_add(fresh) < cap {
            start.push(key);
            fresh += 1;
        } else {
            waiting.push(key);
        }
    }
    LspStartPlan { start, waiting }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(official: &[&str], running: &[&str], cap: u32) -> LspStartInput {
        LspStartInput {
            master_enabled: true,
            lsp_enabled: true,
            checked: vec![
                "rust".into(),
                "go".into(),
                "python".into(),
                "typescript".into(),
            ],
            detected: vec![
                "rust".into(),
                "go".into(),
                "python".into(),
                "typescript".into(),
            ],
            official: official.iter().map(|key| (*key).to_string()).collect(),
            running: running.iter().map(|key| (*key).to_string()).collect(),
            max_concurrent: cap,
        }
    }

    #[test]
    fn empty_official_registry_starts_nothing() {
        let mut sample = input(&[], &[], 2);
        sample.official = official_lsp_provider_ids()
            .iter()
            .map(|key| (*key).to_string())
            .collect();
        let plan = plan_lsp_starts(&sample);
        assert!(plan.start.is_empty());
        assert!(plan.waiting.is_empty());
        assert!(official_lsp_provider_ids().is_empty());
    }

    #[test]
    fn cap_queues_without_evicting_running_providers() {
        let plan = plan_lsp_starts(&input(
            &["rust", "go", "python", "typescript"],
            &["rust", "go"],
            2,
        ));
        assert_eq!(plan.start, vec!["rust", "go"]);
        assert_eq!(plan.waiting, vec!["python", "typescript"]);
    }

    #[test]
    fn free_slot_starts_one_new_language_and_keeps_the_running_one() {
        let mut sample = input(&["rust", "go", "python", "typescript"], &["rust"], 2);
        sample.checked = vec!["go".into(), "python".into(), "rust".into()];
        let plan = plan_lsp_starts(&sample);
        assert_eq!(plan.start, vec!["go", "rust"]);
        assert_eq!(plan.waiting, vec!["python"]);
    }

    #[test]
    fn switches_off_start_nothing_even_with_fake_providers() {
        let mut sample = input(&["rust", "go"], &[], 2);
        sample.master_enabled = false;
        assert!(plan_lsp_starts(&sample).start.is_empty());
        sample.master_enabled = true;
        sample.lsp_enabled = false;
        let plan = plan_lsp_starts(&sample);
        assert!(plan.start.is_empty());
        assert!(plan.waiting.is_empty());
    }

    #[test]
    fn intersection_skips_unchecked_undetected_and_unofficial() {
        let mut sample = input(&["rust", "python"], &[], 8);
        sample.checked = vec!["rust".into(), "go".into(), "python".into()];
        sample.detected = vec!["rust".into(), "go".into()];
        let plan = plan_lsp_starts(&sample);
        assert_eq!(plan.start, vec!["rust"]);
        assert!(plan.waiting.is_empty());
    }
}
