//! mac_control 下沉类型与纯函数（阶段 3：随审批/执行层安全逻辑留 kernel）。
//!
//! `MacControlFocusAnchor`：审批弹窗前后焦点保护的快照类型（execution.rs 持有
//! 跨 await 的 Option<Anchor>，类型必须在 kernel）；
//! `normalize_perform_ax_action`：AX 动作规范化——permission engine 的
//! dangerous 判定消费（审批分类代码不外迁红线）。ha-mac 原路径再导出。

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacControlFocusAnchor {
    pub pid: i32,
    pub bundle_id: Option<String>,
    pub name: Option<String>,
    pub focused_window_id: Option<String>,
    pub focused_window_title: Option<String>,
}

pub fn normalize_perform_ax_action(action: &str) -> Option<String> {
    let action = action.trim();
    if action.is_empty() {
        return None;
    }
    if action.len() > 128
        || !action
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return None;
    }
    let canonical = match action.to_ascii_lowercase().as_str() {
        "press" | "axpress" => Some("AXPress"),
        "show_menu" | "showmenu" | "axshowmenu" => Some("AXShowMenu"),
        "confirm" | "axconfirm" => Some("AXConfirm"),
        "cancel" | "axcancel" => Some("AXCancel"),
        "increment" | "axincrement" => Some("AXIncrement"),
        "decrement" | "axdecrement" => Some("AXDecrement"),
        "pick" | "axpick" => Some("AXPick"),
        "raise" | "axraise" => Some("AXRaise"),
        "show_default_ui" | "showdefaultui" | "axshowdefaultui" => Some("AXShowDefaultUI"),
        "show_alternate_ui" | "showalternateui" | "axshowalternateui" => Some("AXShowAlternateUI"),
        _ => None,
    };
    Some(canonical.unwrap_or(action).to_string())
}
