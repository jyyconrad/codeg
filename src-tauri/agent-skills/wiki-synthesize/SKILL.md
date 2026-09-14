---
name: wiki-synthesize
description: Organize source materials into linked personal Wiki notes; the host adds source metadata.
disable-model-invocation: true
---

# Wiki 综合整理

此提示由 Wiki Worker 的综合任务加载，将当前记忆材料整理为工作、能力和知识笔记。compile 提供来源与项目元数据，代码负责身份、来源关联、提交和去重进度。

Use `source_references` (file paths and source IDs), the existing Wiki index and project/folder metadata to organize the Wiki. Read the supplied materials and existing related pages as needed. Follow links when useful. The host does not require input hashes, read receipts or line-by-line evidence. There is no preliminary segment-summary stage.

Return JSON. The host assigns file names and note IDs, adds YAML and source links, and saves the notes. Do not write published files yourself.

```json
{
  "schema": "codeg.wiki.synthesize.v2",
  "page_proposals": [
    {
      "proposal_key": "pagination-method",
      "op": "create",
      "type": "method",
      "title": "游标分页的边界检查",
      "summary": "整理空页、重复数据与稳定排序的检查方法。",
      "body": "## 检查步骤\n核对连续翻页的顺序、空页和重复数据。",
      "input_rels": ["work/turns/source-example.md"]
    }
  ],
  "warnings": []
}
```

Allowed types: `work-record`, `decision`, `outcome`, `project`, `area`, `capability`, `concept`, `method`, `entity`. Use a readable title and substantive Markdown body. Standard Markdown and Wiki links to existing notes are allowed. `input_rels` identifies relevant supplied materials; when omitted, the host associates the current batch. Sources are attribution metadata, not a demand to prove every statement. Report unavailable materials in warnings and avoid inventing their contents.

For `create`, omit `existing_note_id`. For `update`, supply the existing note ID from the index. Different projects and equal titles do not establish identity. `related_proposal_keys` optionally links other new notes in this response. `supersede` requires an existing note ID and one `replacement_note_id` or `replacement_proposal_key`; the host keeps the previous body and a link to its replacement. Do not delete notes.

An empty `page_proposals` array means these materials add no useful new Wiki content. No `processed_inputs`, content hashes, structured evidence or line ranges are required.

Write useful explanations: context, actions and outcomes for work; purpose, steps and limits for methods; experience and remaining gaps for capabilities. Distinguish source statements from your inferences. Do not invent achievements, authorship or user expertise. Repository metadata describes current local state and supplies association context; it does not establish historical work or grant access to files outside the allowed Wiki scope.
