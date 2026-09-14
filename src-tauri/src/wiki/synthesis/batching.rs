//! 按笔记数量、正文体积和模型上下文预算划分综合整理批次。
//! compile负责读取当前素材并逐批调用Agent；这里只决定批次边界，
//! 不增加预摘要、输入冻结或完整阅读核验步骤。

use super::*;
use crate::agent::context::budget::estimate_tokens_bytes;

const MAX_BATCH_NOTES: usize = 8;
const MAX_BATCH_CHARS: usize = 60_000;

pub(super) fn plan_batches(notes: Vec<LoadedMemory>, input_budget: u64) -> Vec<Vec<LoadedMemory>> {
    let mut batches = Vec::new();
    let mut current = Vec::new();
    let mut chars = 0;
    let mut cost = 0;
    for note in notes {
        let n = note.text.chars().count();
        let tokens = estimate_tokens_bytes(&note.text);
        if !current.is_empty()
            && (current.len() >= MAX_BATCH_NOTES
                || chars + n > MAX_BATCH_CHARS
                || cost + tokens > input_budget)
        {
            batches.push(std::mem::take(&mut current));
            chars = 0;
            cost = 0;
        }
        chars += n;
        cost += tokens;
        current.push(note);
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}
