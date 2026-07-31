//! Slash 命令**契约层**——命令 wire 类型、内置命令静态表、文本解析、
//! 模糊匹配与转录落库，全部零特征依赖。
//!
//! # 为什么与 `slash_commands/` 分家（crate-split 破环）
//!
//! [`crate::slash_commands`] 是**装配层**（composition root）：它的
//! handler 逐个调 skills / channel / cron / dashboard / coding_improvement
//! 等未来特征 crate，出度 30 个模块、入度只有 2 个——与 `app_init` /
//! `globals` 同型，位置在依赖图**顶端**。
//!
//! 但 IM 渠道（未来的 ha-channel）需要的东西里，绝大多数并不是「分发」，
//! 而是这些契约物：命令定义表、`CommandAction` / 各 PickerItem 类型、
//! `is_command` 解析、选择器行渲染、slash 转录落库。它们对特征组零引用，
//! 归位 kernel 后 channel 直接用，只有**真正的分发**（`dispatch` /
//! `im_menu_entries`）才需要经 [`crate::slash_hooks`] 回调装配层。
//!
//! 契约层与装配层的方向红线同 `tool_defs` / `tools`：**`slash_defs` 绝不
//! 依赖 `slash_commands`**。`crate::slash_commands` 门面再导出本模块，
//! 既有 `slash_commands::{types,parser,registry,fuzzy}::…` 路径不变。

pub mod fuzzy;
pub mod history;
pub mod parser;
pub mod registry;
pub mod types;

pub use history::append_slash_history_events;

use types::SessionPickerItem;

/// Resolve silent built-in aliases to their canonical command names for
/// metadata lookup paths (arg options, help text, etc.). Dispatch still matches
/// aliases explicitly so the behavior stays obvious at the side-effect boundary.
pub fn canonical_builtin_command_name(name: &str) -> &str {
    match name {
        "reasoning" => "reason",
        "think" => "thinking",
        _ => name,
    }
}

/// Format one session row for the markdown body of `/sessions`. Shared by
/// the slash handler (GUI markdown) and the channel text-fallback so the
/// two surfaces stay aligned. When the session was matched via message-body
/// FTS, a second indented line shows the matched snippet.
pub(crate) fn format_session_picker_line(s: &SessionPickerItem) -> String {
    let id_short: String = s.id.chars().take(8).collect();
    let mut chips: Vec<String> = Vec::with_capacity(3);
    if !s.agent_label.is_empty() {
        chips.push(format!("agent: {}", s.agent_label));
    }
    if let Some(pl) = s.project_label.as_deref() {
        chips.push(format!("project: {}", pl));
    }
    if let Some(cl) = s.channel_label.as_deref() {
        chips.push(cl.to_string());
    }
    let suffix = if chips.is_empty() {
        String::new()
    } else {
        format!(" · _{}_", chips.join(" · "))
    };
    let head = format!("- `{}` · {}{}", id_short, s.title, suffix);
    match s.snippet.as_deref() {
        Some(sn) if !sn.is_empty() => format!("{}\n  > {}", head, sn),
        _ => head,
    }
}

/// Truncate a description to `max_chars` characters, appending "…" if truncated.
pub(crate) fn truncate_description(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars - 1).collect();
    format!("{}…", truncated)
}

/// IM 菜单的硬上限。Telegram / Discord 的命令列表都有平台配额，超出即
/// 整批注册失败，故本地先截断。
pub const IM_MENU_HARD_CAP: usize = 100;

/// IM 菜单的统一收口：剔除 `IM_DISABLED_COMMANDS`、按 [`IM_MENU_HARD_CAP`]
/// 截断。装配层的 `im_menu_entries` 与 [`crate::slash_hooks`] 的未装配回退
/// 共用，保证两条路径出的菜单口径一致。
pub fn im_menu_filter_and_cap(defs: Vec<types::SlashCommandDef>) -> Vec<types::SlashCommandDef> {
    let mut entries: Vec<types::SlashCommandDef> = defs
        .into_iter()
        .filter(|cmd| !registry::is_im_disabled(&cmd.name))
        .collect();
    if entries.len() > IM_MENU_HARD_CAP {
        crate::app_warn!(
            "channel",
            "menu_sync",
            "Slash command count {} exceeds IM menu cap {} — truncating tail",
            entries.len(),
            IM_MENU_HARD_CAP
        );
        entries.truncate(IM_MENU_HARD_CAP);
    }
    entries
}
