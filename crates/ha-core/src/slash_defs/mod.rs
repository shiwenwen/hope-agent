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

use std::collections::HashSet;
use std::sync::OnceLock;
use types::SessionPickerItem;

/// Collision-resolved typed command name paired with its source entry index.
/// The contract layer stays generic so callers can apply the exact same table
/// to Skills without introducing a kernel -> slash assembly dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDynamicCommandName {
    pub typed_name: String,
    pub entry_index: usize,
}

/// Resolve dynamic command names against the built-in/reserved namespace.
/// The first name of each entry is canonical: collisions gain `_skill`, then
/// `_2`/`_3`/...; colliding aliases are dropped. Inputs are pre-normalized.
pub fn resolve_dynamic_command_names(
    command_names: &[Vec<String>],
    reserved: &HashSet<String>,
) -> Vec<ResolvedDynamicCommandName> {
    let mut used = reserved.clone();
    let mut out = Vec::with_capacity(command_names.len());
    for (entry_index, names) in command_names.iter().enumerate() {
        let Some(canonical) = names.first() else {
            continue;
        };
        let mut display = if used.contains(canonical) {
            format!("{canonical}_skill")
        } else {
            canonical.clone()
        };
        let base = display.clone();
        let mut counter = 2;
        while used.contains(&display) {
            display = format!("{base}_{counter}");
            counter += 1;
        }
        used.insert(display.clone());
        out.push(ResolvedDynamicCommandName {
            typed_name: display,
            entry_index,
        });

        for alias in names.iter().skip(1) {
            if used.insert(alias.clone()) {
                out.push(ResolvedDynamicCommandName {
                    typed_name: alias.clone(),
                    entry_index,
                });
            }
        }
    }
    out
}

/// Dispatcher aliases accepted by handlers but intentionally hidden from the
/// registered command catalog. They still reserve names against Skills.
const SILENT_BUILTIN_ALIASES: &[&str] = &["reasoning", "think"];

/// Built-in command namespace shared by listing, dispatch, and typed binding
/// validation. Kept in the contract layer so lower kernel paths never depend
/// on the `slash_commands` composition root.
pub fn builtin_command_names() -> &'static HashSet<String> {
    static CACHE: OnceLock<HashSet<String>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut names: HashSet<String> = registry::all_commands()
            .into_iter()
            .map(|command| command.name)
            .collect();
        names.extend(
            SILENT_BUILTIN_ALIASES
                .iter()
                .map(|name| (*name).to_string()),
        );
        names
    })
}

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
pub fn format_session_picker_line(s: &SessionPickerItem) -> String {
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
