"""Generate the editable, offline Personal Wiki design sketches. No dependencies."""

from html import escape
from pathlib import Path

ROOT = Path(__file__).parent
INK = "#202023"
MUTED = "#71717a"
LINE = "#e4e4e7"
BG = "#fafafa"


def rect(x, y, w, h, fill="white", stroke=LINE, r=10):
    return f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{fill}" stroke="{stroke}"/>'


def text(x, y, value, size=16, color=INK, weight=400, anchor="start"):
    return f'<text x="{x}" y="{y}" font-size="{size}" fill="{color}" font-weight="{weight}" text-anchor="{anchor}">{escape(value)}</text>'


def lines(x, y, values, size=16, color=INK, step=28, weight=400):
    return "".join(text(x, y + i * step, v, size, color, weight) for i, v in enumerate(values))


def rule(x, y, width):
    return f'<path d="M{x} {y}h{width}" stroke="{LINE}"/>'


def button(x, y, width, label, primary=False, height=40, disabled=False):
    fill = "#e4e4e7" if disabled else INK if primary else "white"
    color = "#a1a1aa" if disabled else "white" if primary else INK
    return rect(x, y, width, height, fill, fill if disabled or primary else LINE, 7) + text(x + width / 2, y + height / 2 + 6, label, 16, color, 500, "middle")


def pill(x, y, label, kind="neutral", width=None):
    palette = {"neutral": ("#f4f4f5", "#52525b"), "success": ("#edf7f0", "#36764b"), "warn": ("#fff5e7", "#906421"), "error": ("#fcf0f0", "#ad4444")}
    fill, color = palette[kind]
    width = width or len(label) * 14 + 20
    return rect(x, y, width, 28, fill, fill, 5) + text(x + 10, y + 19, label, 14, color)


def chevron(x, y, direction="down"):
    path = "m-4 -2 4 4 4 -4" if direction == "down" else "m-2 -4 4 4 -4 4"
    return f'<path d="M{x} {y}{path}" stroke="{MUTED}" stroke-width="1.7" fill="none"/>'


def toggle(x, y):
    return rect(x, y, 42, 24, INK, INK, 12) + f'<circle cx="{x + 30}" cy="{y + 12}" r="8" fill="white"/>'


def search(x=944, y=199, width=448):
    return rect(x, y, width, 44, "white", LINE, 7) + f'<circle cx="{x+22}" cy="{y+20}" r="6" fill="none" stroke="{MUTED}" stroke-width="1.6"/><path d="M{x+26} {y+25}l5 5" stroke="{MUTED}" stroke-width="1.6"/>' + text(x + 44, y + 28, "搜索笔记标题或正文", 15, MUTED)


def shell(name, active="概览"):
    s = '<svg xmlns="http://www.w3.org/2000/svg" width="1440" height="1000" viewBox="0 0 1440 1000" role="img" aria-labelledby="title desc">'
    s += f'<title id="title">个人 Wiki · {escape(name)} · 设计草图</title><desc id="desc">中等保真界面草图，全部内容为示例数据。采用左侧导航与列表、右侧阅读布局。</desc>'
    s += '<g font-family="PingFang SC, Noto Sans CJK SC, Hiragino Sans GB, sans-serif">'
    s += rect(0, 0, 1440, 1000, BG, BG, 0) + rect(0, 0, 208, 1000, "#f4f4f5", LINE, 0)
    s += text(28, 48, "Codeg", 24, INK, 650) + text(28, 106, "工作台", 14, MUTED)
    for y, label in [(150, "对话"), (200, "项目"), (250, "个人 Wiki")]:
        if label == "个人 Wiki":
            s += rect(16, y - 29, 176, 42, "white", LINE, 7)
        s += text(44, y, label, 16, INK, 550 if label == "个人 Wiki" else 400)
    s += rule(24, 898, 160) + text(28, 936, "本地工作区", 14, MUTED)
    s += rect(208, 0, 1232, 164, "white", LINE, 0)
    s += text(240, 49, "个人 Wiki", 20, INK, 600)
    s += button(1134, 22, 136, "＋ 添加资料") + button(1282, 22, 110, "设置")
    for x, label, width in [(240, "概览", 64), (320, "工作", 64), (400, "能力", 64), (480, "资料", 64), (560, "处理记录", 104)]:
        s += text(x + 8, 124, label, 16, INK if active == label else MUTED, 600 if active == label else 400)
        if active == label:
            s += rect(x, 146, width, 3, INK, INK, 1)
    s += text(240, 978, "设计草图 · 示例数据", 14, MUTED)
    s += text(1392, 978, name, 14, MUTED, anchor="end")
    return s


def write(name, title, body):
    (ROOT / name).write_text(body + "</g></svg>\n", encoding="utf-8")
    return {"file": name, "title": title}


pages = []

# 01 — overview, with an actionable recovery notice.
s = shell("概览")
s += text(240, 226, "个人 Wiki", 28, INK, 650) + text(240, 257, "回看做过的工作，积累可复用的方法。", 16, MUTED) + search()
for x, count, title, note in [(240, "12", "可读笔记", "工作与能力笔记"), (632, "2", "待处理", "任务 1 · 待归纳记录 1"), (1024, "3", "需要处理", "尚未解决的问题，按处理对象去重")]:
    s += rect(x, 284, 368, 100) + text(x + 20, 321, title, 15, MUTED) + text(x + 20, 361, count, 28, INK, 600) + text(x + 71, 358, note, 14, MUTED)
s += rect(240, 408, 1152, 88, "#fffaf1", "#ead9b9", 8)
s += text(264, 438, "部分历史记录未生成笔记", 17, INK, 600) + text(264, 469, "原始材料仍在，可重新生成。", 16, MUTED)
s += text(997, 457, "3 条待核对", 15, "#906421") + button(1136, 432, 232, "查看并恢复")
s += text(240, 540, "最近更新", 20, INK, 600) + text(1392, 540, "示例计数；待处理不包含仅归档资料", 14, MUTED, anchor="end")
s += rect(240, 562, 436, 356) + rect(692, 562, 700, 356)
recent = [("补齐接口分页规则", "明确了游标参数、排序规则和边界行为。", "工作 · 示例项目 · 今天 10:24"), ("接口设计", "整理分页接口的设计方法与实践记录。", "能力 · 今天 09:10"), ("收敛错误提示文案", "统一三类失败提示与重试入口。", "工作 · 昨天 16:35")]
for i, (title, desc, meta) in enumerate(recent):
    y = 574 + i * 112
    if i == 0:
        s += rect(252, y, 412, 102, "#f4f4f5", "#f4f4f5", 6)
    s += text(268, y + 28, title, 17, INK, 600) + text(268, y + 55, desc, 15, MUTED) + text(268, y + 81, meta, 14, MUTED)
    if i < 2:
        s += rule(268, y + 107, 372)
s += pill(720, 586, "单轮记录") + text(720, 650, "补齐接口分页规则", 24, INK, 600)
s += lines(720, 689, ["明确了游标参数、排序规则和边界行为，", "便于后续实现与核对。"], 17, MUTED)
s += rule(720, 742, 644) + text(720, 781, "这次做了什么", 18, INK, 600) + text(720, 815, "梳理游标传递方式，补充排序与空结果的处理规则。", 16)
s += button(720, 850, 136, "阅读笔记", True)
pages.append(write("01-overview.svg", "概览", s))

# 02 — onboarding within the actual Wiki shell.
s = shell("首次使用")
s += pill(240, 198, "尚未启用")
s += rect(240, 246, 1152, 640)
s += text(816, 321, "把工作记录变成可回看的笔记", 30, INK, 650, "middle")
s += text(816, 363, "会话结束后自动记录工作；你也可以添加资料，随时阅读原文。", 18, MUTED, anchor="middle")
s += button(628, 406, 184, "启用个人 Wiki", True, 46) + button(828, 406, 180, "先了解如何使用", False, 46)
for x, number, title, desc in [(288, "1", "记录工作", ["保存每轮工作的要点，", "会话完成后汇总过程与结果。"]), (656, "2", "归纳经验", ["从工作笔记中归纳方法，", "积累以后能用上的经验。"]), (1024, "3", "保存资料", ["添加文件或已有记录，", "在资料页直接阅读原文。"])]:
    s += rect(x, 510, 320, 230, "#fafafa", LINE, 8) + pill(x + 24, 534, number, width=32)
    s += text(x + 24, 602, title, 21, INK, 600) + lines(x + 24, 642, desc, 16, MUTED)
s += text(816, 798, "添加的资料只归档供阅读，不会自动生成工作或能力笔记。", 16, MUTED, anchor="middle")
s += text(816, 841, "启用后可在设置中调整自动归纳时间和模型。", 15, MUTED, anchor="middle")
pages.append(write("02-first-use.svg", "首次使用", s))

# 03 — readable note, metadata and source controls kept separate.
s = shell("工作笔记阅读", "工作")
s += text(240, 220, "工作", 26, INK, 650) + search(944, 190)
s += rect(240, 250, 340, 680) + rect(596, 250, 796, 680)
s += text(264, 286, "示例项目", 17, INK, 600) + chevron(550, 280) + rule(264, 309, 292)
for i, (title, desc, meta) in enumerate([("补齐接口分页规则", "明确参数、排序和边界行为", "单轮记录 · 今天 10:24"), ("分页接口设计回顾", "从需求确认到验收的对话总结", "对话总结 · 昨天 18:00"), ("收敛错误提示文案", "统一失败提示与重试入口", "单轮记录 · 昨天 16:35")]):
    y = 325 + 117 * i
    if i == 0:
        s += rect(252, y, 316, 104, "#f4f4f5", "#f4f4f5", 6)
    s += text(268, y + 29, title, 17, INK, 600) + text(268, y + 57, desc, 15, MUTED) + text(268, y + 83, meta, 14, MUTED)
s += text(628, 286, "示例项目", 15, MUTED) + button(1152, 268, 152, "查看来源") + button(1316, 268, 44, "···")
s += pill(628, 322, "单轮记录") + pill(734, 322, "助手报告")
s += text(628, 394, "补齐接口分页规则", 29, INK, 650)
s += lines(628, 435, ["明确了游标参数、排序规则和边界行为，", "便于后续实现与核对。"], 17, MUTED)
s += text(628, 502, "今天 10:24 更新", 14, MUTED) + rule(628, 526, 732)
s += text(628, 566, "这次做了什么", 20, INK, 600)
s += lines(628, 603, ["•  约定列表按更新时间降序，时间相同则按唯一键排序。", "•  使用上一页末项生成游标；首屏请求不传游标。", "•  补充空结果、无效游标和最后一页的返回规则。"], 17, step=32)
s += text(628, 714, "结果与待办", 20, INK, 600)
s += lines(628, 751, ["已整理接口约定，后续可据此实现并核对。", "待办：补齐分页边界用例，并确认并发更新时的结果一致性。"], 17, step=32)
s += text(628, 839, "相关资料", 20, INK, 600) + text(628, 875, "分页参数讨论记录  ↗", 17)
s += text(1337, 916, "··· 菜单：查看源码", 14, MUTED, anchor="end")
pages.append(write("03-note-reader.svg", "工作笔记", s))

# 04 — archive source shows extracted content first.
s = shell("资料原文阅读", "资料")
s += text(240, 220, "资料", 26, INK, 650) + text(328, 219, "归档材料，随时阅读全文", 16, MUTED)
s += rect(240, 250, 340, 680) + rect(596, 250, 796, 680)
s += text(264, 286, "已保存资料", 17, INK, 600) + rule(264, 309, 292)
for i, (title, detail) in enumerate([("接口设计参考", "文件 · 今天 09:50"), ("需求讨论摘录", "粘贴文本 · 昨天 15:20"), ("历史对话记录", "历史对话 · 昨天 10:08")]):
    y = 325 + 105 * i
    if i == 0:
        s += rect(252, y, 316, 91, "#f4f4f5", "#f4f4f5", 6)
    s += text(268, y + 32, title, 17, INK, 600) + text(268, y + 62, detail, 14, MUTED)
s += text(628, 294, "接口设计参考", 26, INK, 650) + pill(1196, 268, "仅归档", width=88)
s += lines(628, 333, ["资料已保存，可阅读全文。", "它不会自动进入工作与能力归纳。"], 16, MUTED)
for x, label in [(628, "提取正文"), (772, "资料信息"), (916, "关联笔记")]:
    s += text(x, 408, label, 16, INK if x == 628 else MUTED, 600 if x == 628 else 400)
s += rule(628, 430, 732) + rect(628, 428, 72, 3, INK, INK, 1)
s += text(628, 478, "分页接口约定", 24, INK, 600)
s += lines(628, 518, ["列表接口使用游标分页。调用方保存本次响应中的下一页游标，", "并在下次请求中传入。未返回下一页游标时，表示结果已取完。"], 17, step=30)
s += text(628, 602, "请求参数", 20, INK, 600)
s += rect(628, 624, 732, 158, "white", LINE, 4) + rect(628, 624, 732, 42, "#f4f4f5", LINE, 4)
s += text(646, 652, "参数", 15, INK, 600) + text(831, 652, "用途", 15, INK, 600) + text(1160, 652, "是否必填", 15, INK, 600)
for y, a, b, c in [(700, "游标", "指定下一页的起点", "否"), (752, "每页数量", "限制本次返回的记录数", "否")]:
    s += text(646, y, a, 16) + text(831, y, b, 16) + text(1160, y, c, 16)
s += rule(628, 722, 732)
s += text(628, 832, "关联笔记", 17, INK, 600) + text(628, 866, "这份资料暂未关联笔记。", 16, MUTED)
s += text(628, 902, "示意：关联笔记为空时的提示", 14, MUTED)
pages.append(write("04-source-reader.svg", "资料原文", s))

# 05 — outcomes, readable failure and result navigation.
s = shell("处理记录与失败详情", "处理记录")
s += text(240, 220, "处理记录", 26, INK, 650) + button(1256, 190, 136, "立即整理", True)
for x, label, width, selected in [(240, "全部", 72, True), (322, "进行中", 90, False), (422, "需要处理", 108, False), (540, "已完成", 90, False)]:
    s += button(x, 250, width, label, selected, 36)
s += rect(240, 310, 512, 620) + rect(768, 310, 624, 620)
jobs = [("生成单轮记录 · 补齐接口分页规则", "读取材料失败", "error", "今天 10:26 · 本次未生成笔记"), ("生成单轮记录 · 收敛错误提示文案", "已生成 · 1 篇笔记", "success", "今天 09:42"), ("归纳方法 · 示例项目", "进行中", "neutral", "今天 09:40 · 正在归纳工作笔记"), ("生成单轮记录 · 简短确认", "无需生成", "neutral", "昨天 18:10 · 本轮没有可记录的新内容")]
for i, (title, status, kind, meta) in enumerate(jobs):
    y = 324 + i * 138
    if i == 0:
        s += rect(252, y, 488, 124, "#faf6f6", "#efdcdc", 6)
    s += text(268, y + 29, title, 17, INK, 600) + pill(268, y + 47, status, kind) + text(268, y + 101, meta, 14, MUTED)
    if i == 1:
        s += text(628, y + 66, "阅读笔记 ↗", 15, INK, 500)
    if i < 3:
        s += rule(268, y + 130, 452)
s += text(800, 354, "生成单轮记录", 17, MUTED) + text(800, 393, "补齐接口分页规则", 25, INK, 600)
s += pill(800, 419, "读取材料失败", "error") + text(800, 483, "未能读取原始材料，本次没有生成笔记。", 17)
s += text(800, 518, "可先查看原始材料，确认可读取后再重试。", 16, MUTED)
s += button(800, 552, 100, "重试", True) + button(912, 552, 180, "查看原始材料")
s += rule(800, 622, 560)
s += text(800, 660, "所属项目", 15, MUTED) + text(970, 660, "示例项目", 16)
s += text(800, 702, "最后处理", 15, MUTED) + text(970, 702, "今天 10:26", 16)
s += text(800, 744, "生成结果", 15, MUTED) + text(970, 744, "未生成笔记", 16)
s += rule(800, 778, 560) + chevron(810, 810, "right") + text(832, 816, "技术详情", 16) + text(832, 847, "错误代码与处理日志", 14, MUTED)
s += text(800, 902, "记录状态变化后，列表与详情同步更新。", 14, MUTED)
pages.append(write("05-processing.svg", "处理记录", s))

# 06 — compact settings, with technical configuration collapsed.
s = shell("基础设置与高级折叠")
s += text(240, 220, "个人 Wiki 设置", 26, INK, 650) + text(240, 253, "决定如何记录工作、何时归纳，以及使用哪个模型。", 16, MUTED)
s += rect(240, 282, 1152, 476)
s += text(272, 328, "启用个人 Wiki", 19, INK, 600) + toggle(1318, 304)
s += text(272, 360, "启用后记录新工作，并可添加资料供阅读。", 16, MUTED) + rule(272, 390, 1088)
s += text(272, 430, "自动归纳", 19, INK, 600) + toggle(1318, 406)
s += text(272, 463, "定时把新的工作笔记归纳为方法与能力笔记。", 16, MUTED)
s += text(272, 518, "整理时间", 16, INK, 500) + button(1016, 488, 344, "每天 03:00（Asia/Shanghai）")
s += chevron(1338, 508) + rule(272, 558, 1088)
s += text(272, 601, "用于整理的模型", 19, INK, 600) + button(1016, 575, 344, "示例模型 A") + chevron(1338, 595)
s += text(272, 639, "启用时复制当前 Codeg Agent 的模型选择，此后独立保存。", 16, MUTED)
s += text(272, 675, "记录工作、对话总结与归纳默认使用同一模型。", 16, MUTED)
s += text(272, 714, "需要分别配置时，可展开高级设置。", 15, MUTED)
s += rect(240, 778, 1152, 70) + chevron(271, 812, "right") + text(296, 820, "高级设置：分阶段模型、提示词、保存位置、排除范围", 17, INK, 500)
s += button(1256, 878, 136, "保存设置", True)
pages.append(write("06-settings.svg", "设置", s))

# 07 — a bounded, previewed recovery operation and its completion state.
s = shell("历史记录恢复预览", "处理记录")
s += text(240, 220, "恢复未生成的笔记", 26, INK, 650)
s += text(240, 256, "重新处理你选中的历史记录，原始材料必须仍在。", 16, MUTED)
s += rect(240, 284, 752, 634) + rect(1012, 284, 380, 338)
s += text(268, 325, "3 条建议恢复", 22, INK, 600) + text(964, 325, "预览结果 · 示例", 14, MUTED, anchor="end")
s += rule(268, 348, 696)
for i, (title, meta) in enumerate([("补齐接口分页规则", "示例项目 · 今天 10:24"), ("完善空结果处理", "示例项目 · 昨天 16:20"), ("核对错误提示文案", "示例项目 · 昨天 14:05")]):
    y = 374 + i * 128
    s += rect(268, y + 7, 20, 20, "white", "#b4b4bb", 4)
    s += text(304, y + 24, title, 18, INK, 600) + text(304, y + 56, meta, 14, MUTED)
    s += pill(800, y + 3, "建议恢复", "neutral", 140) + text(800, y + 57, "原始材料仍在", 15, MUTED)
    if i < 2:
        s += rule(268, y + 94, 696)
s += text(268, 749, "已选择 0 条", 16, INK, 600)
s += rule(268, 770, 696) + lines(268, 810, ["保留原始材料和已有笔记，不会覆盖手动修改的内容。"], 16, MUTED)
s += button(268, 844, 136, "开始恢复", True, disabled=True) + button(416, 844, 136, "暂不恢复")
s += text(1036, 327, "本次恢复范围", 19, INK, 600)
s += lines(1036, 369, ["默认不选择记录。", "选择后才可开始恢复。"], 16, MUTED)
s += rule(1036, 425, 332)
s += pill(1036, 447, "需核对", "warn")
s += lines(1036, 501, ["只有警告、原因不确定的记录，", "先核对原始材料与处理详情。", "已有笔记、手动修改均保留。"], 16, step=34)
s += rect(1012, 642, 380, 276, "#f8fbf8", "#dfe9e0", 10)
s += text(1036, 680, "完成状态示意", 16, MUTED) + text(1036, 722, "恢复处理已结束", 22, INK, 600)
s += lines(1036, 759, ["已生成 3 篇笔记。", "0 条仍需处理。"], 17, step=31)
s += button(1036, 836, 156, "查看新笔记", True)
pages.append(write("07-recovery.svg", "恢复预览", s))

# 08 — capability as grounded practice, no unsupported ranking.
s = shell("能力笔记阅读", "能力")
s += text(240, 220, "能力", 26, INK, 650) + search(944, 190)
s += rect(240, 250, 340, 680) + rect(596, 250, 796, 680)
s += text(264, 286, "可复用的方法", 17, INK, 600) + rule(264, 309, 292)
s += rect(252, 325, 316, 112, "#f4f4f5", "#f4f4f5", 6) + text(268, 356, "接口设计", 18, INK, 600)
s += text(268, 387, "分页约定、边界核对与实践记录", 15, MUTED) + text(268, 417, "今天 09:10 更新", 14, MUTED)
s += text(268, 480, "错误处理", 18, INK, 600) + text(268, 512, "让失败信息可以被理解和处理", 15, MUTED)
s += pill(628, 280, "学习参考") + button(1152, 268, 152, "查看来源") + button(1316, 268, 44, "···")
s += text(628, 357, "接口设计", 29, INK, 650) + text(628, 395, "从具体工作中积累接口约定与核对方法。", 17, MUTED)
s += rule(628, 424, 732) + text(628, 465, "方法与检查点", 20, INK, 600)
s += lines(628, 503, ["先明确输入、返回值和排序规则，再逐项核对边界行为。", "分页接口需同时约定游标生成方式与终止条件。"], 17, step=30)
s += text(628, 590, "实践记录", 20, INK, 600) + text(628, 627, "补齐接口分页规则  ↗", 17, INK, 500)
s += text(628, 659, "助手报告 · 本人角色未说明", 16, MUTED)
s += text(628, 690, "示例项目 · 已整理游标、排序与空结果规则。", 16, MUTED)
s += text(628, 743, "当前边界", 20, INK, 600) + text(628, 780, "目前依据设计讨论；尚未通过并发更新场景验证。", 17)
s += text(628, 843, "下一次实践", 20, INK, 600) + text(628, 880, "实现后增加边界用例，并复核结果一致性。", 17)
s += text(1337, 914, "··· 菜单：查看源码", 14, MUTED, anchor="end")
pages.append(write("08-capability-reader.svg", "能力笔记", s))

# 09 — one clear archive-only import entry.
s = shell("统一添加资料入口", "资料")
s += text(240, 220, "资料", 26, INK, 650)
s += rect(240, 250, 1152, 680) + text(272, 292, "已保存资料", 18, INK, 600)
s += text(272, 343, "接口设计参考", 17) + text(272, 373, "文件 · 今天 09:50", 14, MUTED)
s += '<rect x="208" y="164" width="1232" height="794" fill="#18181b" opacity="0.18"/>'
s += rect(406, 246, 800, 626, "white", "#d4d4d8", 12)
s += text(438, 293, "添加资料", 25, INK, 650) + text(1168, 289, "×", 24, MUTED, anchor="middle")
s += text(438, 331, "添加的资料只归档供阅读，不会自动生成工作或能力笔记。", 16, MUTED)
for x, label, width, active in [(438, "文件", 110, True), (560, "粘贴", 110, False), (682, "历史对话", 144, False), (838, "目录", 110, False)]:
    s += button(x, 365, width, label, active)
s += rect(438, 429, 736, 260, "#fafafa", "#d4d4d8", 8)
s += text(806, 491, "选择要保存的资料", 22, INK, 600, "middle")
s += text(806, 531, "保存后可在资料页阅读提取正文。", 17, MUTED, anchor="middle")
s += button(732, 570, 148, "选择文件") + text(806, 654, "尚未选择文件", 15, MUTED, anchor="middle")
s += rect(438, 711, 736, 60, "#f4f4f5", "#f4f4f5", 6)
s += text(458, 748, "保存后可在资料页阅读原文。重复内容不会重复添加。", 16, MUTED)
s += button(930, 800, 110, "取消") + button(1052, 800, 122, "添加资料", True, disabled=True)
pages.append(write("09-add-source.svg", "添加资料", s))

navigation = "\n".join(f'<a class="nav-link" data-page="{p["file"][:-4]}" href="#{p["file"][:-4]}">{i+1:02d} {p["title"]}</a>' for i, p in enumerate(pages))
sections = "\n".join(f'<section class="screen" id="{p["file"][:-4]}" aria-label="{p["title"]}"><div class="screen-label"><h2>{p["title"]}</h2><div><a href="{p["file"]}">打开 SVG</a><a href="{p["file"][:-4]}.png">打开 PNG</a></div></div><img src="{p["file"]}" width="1440" height="1000" alt="个人 Wiki {p["title"]}设计草图，示例数据"></section>' for p in pages)
(ROOT / "index.html").write_text('''<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>个人 Wiki 修复方案 · 界面草图</title>
<style>
*{box-sizing:border-box}body{margin:0;background:#ededee;color:#202023;font-family:"PingFang SC","Noto Sans CJK SC",sans-serif;font-size:16px}header{background:#fff;border-bottom:1px solid #ddd;padding:24px 32px}h1{margin:0 0 10px;font-size:24px}p{margin:0;color:#71717a;line-height:1.7}nav{display:flex;flex-wrap:wrap;gap:8px;margin-top:20px}a{color:inherit;text-decoration:none}.nav-link{padding:9px 14px;border:1px solid #e4e4e7;border-radius:6px;background:#fff;font-size:15px}.nav-link.active{background:#202023;color:white;border-color:#202023}.screen{display:none;max-width:1488px;margin:0 auto;padding:20px 24px 32px}.screen.active{display:block}.screen-label{display:flex;align-items:center;justify-content:space-between;margin-bottom:14px;gap:20px}h2{margin:0;font-size:18px}.screen-label div{display:flex;gap:18px;font-size:14px;text-decoration:underline}img{display:block;width:100%;height:auto;border:1px solid #d4d4d8;background:#fff;box-shadow:0 3px 15px #00000008}footer{max-width:1488px;margin:0 auto;padding:0 24px 24px;font-size:14px;color:#71717a;line-height:1.8}@media(max-width:720px){header{padding:20px 16px}.screen{padding:16px 8px}h1{font-size:21px}img{min-width:1100px}.screen{overflow-x:auto}.screen-label{min-width:500px}footer{padding:0 16px 24px}}
</style></head><body><header><h1>个人 Wiki 修复方案 · 界面草图</h1><p>中等保真方案，共 9 个界面。全部内容和数量为示例，不代表实际数据或修复结果。点击导航切换；产品内按钮为静态示意。</p><nav aria-label="草图导航">''' + navigation + '''</nav></header><main>''' + sections + '''</main><footer>草图尺寸 1440 × 1000，最小字号 14。离线使用，无网络请求。恢复页右侧的“完成状态示意”是另一状态的设计说明。</footer><script>
function showPage(){const id=location.hash.slice(1)||'01-overview';const exists=document.getElementById(id);const chosen=exists?id:'01-overview';document.querySelectorAll('.screen').forEach(el=>el.classList.toggle('active',el.id===chosen));document.querySelectorAll('.nav-link').forEach(el=>{el.classList.toggle('active',el.dataset.page===chosen);if(el.dataset.page===chosen)el.setAttribute('aria-current','page');else el.removeAttribute('aria-current')})}window.addEventListener('hashchange',showPage);showPage();
</script></body></html>
''', encoding="utf-8")

print(f"Generated {len(pages)} SVG sketches and index.html in {ROOT}")
