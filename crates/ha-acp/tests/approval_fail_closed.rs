//! ACP 审批 fail-closed 红线（AGENTS.md「无人值守 fail-closed」段 +
//! `crates/ha-acp/src/acp/agent.rs:759-770` 的自述）。
//!
//! ACP 模式（stdio 桥接编辑器客户端）当前**没有**外出的
//! `session/request_permission` 通道——需审批的工具无法把 prompt 送到编辑器。
//! `permission::evaluate_approval_surface` 因此在 ACP 模式 + 客户端未声明
//! permission capability 时返回 `Unattended(AcpNoPermissionCapability)`，
//! 上层由 `unattended_approval_action`（默认 deny）落地为拒绝，而不是让 prompt
//! 悬挂到超时。客户端一旦通过 `set_acp_permission_capable(true)` 声明能力
//! （D7 initialize 时机），surface 立即回到 `Attended`。
//!
//! **为什么放这里（ha-acp/tests/ 而不是 kernel 里的 mod tests）**：kernel 里
//! `RUNTIME_ROLE` 是 `OnceLock<&'static str>`，第一次 set 后不可改；ha-core 的
//! 单元测试共享一个 test binary，早已被别的 test 或初始化路径固化为非 "acp"
//! 值。开一个 ha-acp integration test binary（自带独立 `OnceLock`），才能
//! 干净地把 role 设成 "acp" 触发对应分支——这也是 ha-updater / ha-mcp /
//! ha-server 已确立的模式（每个特征 crate 的 e2e 集成测试都用独立 binary）。

use ha_core::permission::{
    approval_surface::set_acp_permission_capable, evaluate_approval_surface, ApprovalSurface,
    UnattendedReason,
};

/// **AGENTS.md 无人值守 fail-closed 红线**：ACP 模式无 permission capability
/// 的默认状态必须表现为 `Unattended(AcpNoPermissionCapability)`，让调用侧
/// 应用 `unattended_approval_action`（默认 deny）拒掉需审批的工具，而不是
/// 走进 desktop / attended 分支或 headless 分支被误判。
///
/// 顺带覆盖 D7 capability toggle：客户端声明后回到 `Attended`。这条 toggle
/// 在 kernel `approval_surface::tests::acp_capability_toggle_flips_acp_surface`
/// 单独测过 pure round-trip，但那条 test 没 set role="acp"，所以并没有真的走
/// `evaluate_approval_surface` 里的 ACP 分支——本 test 补齐这个 end-to-end
/// 组合（role="acp" ∧ capability=false → Unattended；capability=true → Attended）。
#[test]
fn acp_mode_without_permission_capability_yields_unattended_deny() {
    // ACP integration test binary → 独占的 `RUNTIME_ROLE` `OnceLock`，可以
    // 干净设成 "acp"。之后本 binary 内其它 test 若加，仍处 "acp" 模式（幂等
    // OnceLock），语义与 `hope-agent acp` 真运行时一致。
    ha_base::runtime_role::set_runtime_role("acp");
    assert!(
        ha_base::runtime_role::is_acp(),
        "runtime role must be 'acp' for this suite; is_acp() must reflect it \
         so evaluate_approval_surface takes the ACP branch"
    );

    // Default 状态：ACP 客户端还没调 `set_acp_permission_capable(true)`
    // （D7 initialize 时才会调）。session_id=None 走顶层 turn 路径，跳过
    // cron/subagent/IM/desktop 前 4 步，命中步骤 5 的 ACP 分支。
    set_acp_permission_capable(false);
    let surface = evaluate_approval_surface(None);
    assert_eq!(
        surface,
        ApprovalSurface::Unattended(UnattendedReason::AcpNoPermissionCapability),
        "ACP without permission-forwarding capability must classify as Unattended \
         so the caller applies unattendedApprovalAction (default deny) — otherwise \
         a tool needing approval would hang the prompt until timeout, exactly what \
         `agent.rs:759-770` fail-closes against."
    );

    // D7: 客户端 initialize 时声明了 permission capability → surface 回到
    // Attended，正常审批流。
    set_acp_permission_capable(true);
    let surface = evaluate_approval_surface(None);
    assert_eq!(
        surface,
        ApprovalSurface::Attended,
        "ACP with declared permission capability must be Attended so approval \
         prompts forward to the editor (D7)"
    );

    // 清理：恢复 default false，避免影响后续新增 test（本 binary 内共享）。
    set_acp_permission_capable(false);
}
