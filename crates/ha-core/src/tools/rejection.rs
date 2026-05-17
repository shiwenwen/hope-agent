//! 统一的"工具被用户拒绝/取消"语义。
//!
//! 将 deny / approval-timeout / cancel 三类终止统一为类型化错误，
//! 在 [`crate::agent::streaming_loop`] 出口处 downcast 后渲染成给 LLM
//! 的 `tool_result` 文本——确保：
//!   1. 文本始终带 `Tool error:` 前缀，触发 `is_error` 通道（UI 标红、warn 日志）；
//!   2. 文本以 "STOP and wait" 收尾，避免模型把拒绝当成可重试的失败再调一次；
//!   3. 副作用句按变体区分——`Deny` / `ApprovalTimeout` 在 permission gate 阶段返回
//!      （tool body 未跑）声明 "no side effects"；`Cancelled` 是 select! drop 已经
//!      在跑的 future（子进程 / `fs::write` 已发生且无法回滚）改为 "may have
//!      completed"，避免误导模型假设回滚。
//!
//! 重启恢复用的 `INTERRUPTED_TOOL_RESULT` ([`crate::chat_engine::context`])
//! **不**走这条路径——其语义是"上轮被打断、move on"，与 STOP 相反。
//!
//! `ask_user_question` 取消同样**不**走这条路径——它是 tool 自身 `Ok` 返回，
//! 模型本来就该停下等下一轮用户输入，加 `Tool error:` 反而把 UI 标红，
//! 与"用户主动选择不答"语义不符。

use std::fmt;

/// 所有 LLM 可见 tool_result 错误文本的统一前缀。
/// [`crate::agent::streaming_loop`] 用 `starts_with` 判 `is_error`。
pub const TOOL_ERROR_PREFIX: &str = "Tool error: ";

/// 统一的"工具被用户拒绝/取消"原因。
#[derive(Debug, Clone)]
pub enum ToolRejection {
    /// 用户在审批弹窗点击 Deny。
    DeniedByUser { name: String },
    /// 权限引擎判定拒绝（保护路径 / 危险命令 / Smart judge 等）。
    DeniedByPolicy { name: String, reason: String },
    /// 审批通道异常（非用户拒绝、非配置化超时），为了避免 fail-open
    /// 默认阻止工具执行。
    ApprovalFailed { name: String, error: String },
    /// 审批弹窗超时且 `approval_timeout_action=deny`。
    ApprovalTimeout { name: String, timeout_secs: u64 },
    /// 用户在工具执行期间取消整个 turn。
    Cancelled { name: String },
}

impl ToolRejection {
    /// 渲染成给 LLM 看的 `tool_result` 文本——[`TOOL_ERROR_PREFIX`] 触发
    /// `is_error` 通道，"STOP and wait" 后缀阻止模型把拒绝当成可重试错误。
    ///
    /// `Deny` / `ApprovalFailed` / `ApprovalTimeout` 在 permission gate 阶段返回，
    /// tool body 未跑——文案声明 "no side effects"。`Cancelled` 是 select! drop
    /// 已经在跑的 future，子进程 / `fs::write` / 网络调用可能已经发生且无法回滚——
    /// 文案改为 "may have completed"，避免误导模型假设回滚或重试。
    pub fn to_tool_result(&self) -> String {
        let side_effect_clause = match self {
            Self::Cancelled { .. } => {
                "Any side effects already in progress may have completed and cannot be assumed \
                 to be rolled back"
            }
            Self::DeniedByUser { .. }
            | Self::DeniedByPolicy { .. }
            | Self::ApprovalFailed { .. }
            | Self::ApprovalTimeout { .. } => {
                "The tool did not execute and no side effects occurred"
            }
        };
        format!(
            "{TOOL_ERROR_PREFIX}{self}. {side_effect_clause}. \
             STOP what you are doing and wait for the user to tell you how to proceed."
        )
    }

    /// 把任意 `Err` 渲染成 LLM 可见的 tool_result。
    ///
    /// `ToolRejection` 走专属模板；其它 `anyhow::Error` 走 `Tool error: <msg>`。
    pub fn render_error(e: &anyhow::Error) -> String {
        match e.downcast_ref::<Self>() {
            Some(rej) => rej.to_tool_result(),
            None => format!("{TOOL_ERROR_PREFIX}{e}"),
        }
    }

    pub fn denied_by_user(name: impl Into<String>) -> anyhow::Error {
        Self::DeniedByUser { name: name.into() }.into()
    }

    pub fn denied_by_policy(name: impl Into<String>, reason: impl Into<String>) -> anyhow::Error {
        Self::DeniedByPolicy {
            name: name.into(),
            reason: reason.into(),
        }
        .into()
    }

    pub fn approval_timeout(name: impl Into<String>, timeout_secs: u64) -> anyhow::Error {
        Self::ApprovalTimeout {
            name: name.into(),
            timeout_secs,
        }
        .into()
    }

    pub fn approval_failed(name: impl Into<String>, error: impl Into<String>) -> anyhow::Error {
        Self::ApprovalFailed {
            name: name.into(),
            error: error.into(),
        }
        .into()
    }

    pub fn cancelled(name: impl Into<String>) -> Self {
        Self::Cancelled { name: name.into() }
    }
}

impl fmt::Display for ToolRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeniedByUser { name } => {
                write!(f, "Tool '{name}' execution denied by user")
            }
            Self::DeniedByPolicy { name, reason } => {
                write!(f, "Tool '{name}' denied: {reason}")
            }
            Self::ApprovalFailed { name, error } => write!(
                f,
                "Tool '{name}' execution blocked: approval check failed ({error})"
            ),
            Self::ApprovalTimeout { name, timeout_secs } => write!(
                f,
                "Tool '{name}' execution denied: approval timed out after {timeout_secs}s"
            ),
            Self::Cancelled { name } => {
                write!(f, "Tool '{name}' execution was cancelled by the user")
            }
        }
    }
}

impl std::error::Error for ToolRejection {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denied_by_user_renders_full_template() {
        let r = ToolRejection::DeniedByUser {
            name: "exec".into(),
        };
        let s = r.to_tool_result();
        assert!(s.starts_with("Tool error: "), "needs Tool error: prefix");
        assert!(s.contains("Tool 'exec' execution denied by user"));
        assert!(s.contains("did not execute and no side effects occurred"));
        assert!(s.contains("STOP what you are doing and wait"));
    }

    #[test]
    fn denied_by_policy_carries_reason() {
        let r = ToolRejection::DeniedByPolicy {
            name: "write".into(),
            reason: "protected path".into(),
        };
        let s = r.to_tool_result();
        assert!(s.contains("Tool 'write' denied: protected path"));
        assert!(s.starts_with("Tool error: "));
    }

    #[test]
    fn approval_timeout_carries_seconds() {
        let r = ToolRejection::ApprovalTimeout {
            name: "exec".into(),
            timeout_secs: 300,
        };
        let s = r.to_tool_result();
        assert!(s.contains("approval timed out after 300s"));
    }

    #[test]
    fn approval_failed_blocks_without_side_effects() {
        let r = ToolRejection::ApprovalFailed {
            name: "exec".into(),
            error: "approval channel closed".into(),
        };
        let s = r.to_tool_result();
        assert!(s.contains("approval check failed (approval channel closed)"));
        assert!(s.contains("did not execute and no side effects occurred"));
        assert!(s.contains("STOP what you are doing and wait"));
    }

    #[test]
    fn cancelled_does_not_claim_no_side_effects() {
        let r = ToolRejection::Cancelled {
            name: "exec".into(),
        };
        let s = r.to_tool_result();
        assert!(s.starts_with("Tool error: "));
        assert!(s.contains("Tool 'exec' execution was cancelled by the user"));
        assert!(s.contains("STOP what you are doing and wait"));
        assert!(
            !s.contains("did not execute and no side effects"),
            "cancel may fire mid-execution; cancel result must NOT promise no side effects"
        );
        assert!(
            s.contains("may have completed"),
            "cancel result must signal that in-flight side effects may have completed"
        );
    }

    #[test]
    fn anyhow_downcast_recovers_typed_rejection() {
        let err: anyhow::Error = ToolRejection::DeniedByUser {
            name: "exec".into(),
        }
        .into();
        let r = err.downcast_ref::<ToolRejection>().expect("downcast");
        assert!(matches!(r, ToolRejection::DeniedByUser { .. }));
    }

    #[test]
    fn render_error_uses_rejection_template_when_typed() {
        let e = ToolRejection::denied_by_user("exec");
        let s = ToolRejection::render_error(&e);
        assert!(s.contains("STOP what you are doing"));
        assert!(s.contains("Tool 'exec' execution denied by user"));
    }

    #[test]
    fn render_error_falls_back_for_plain_anyhow() {
        let e = anyhow::anyhow!("disk full");
        let s = ToolRejection::render_error(&e);
        assert_eq!(s, "Tool error: disk full");
    }
}
