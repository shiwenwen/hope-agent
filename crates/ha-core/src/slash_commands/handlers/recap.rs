use crate::slash_defs::types::CommandResult;

/// `/recap` 的装配层 trampoline。参数解析 / `RecapContext` 构建 / 后台
/// 生成 / `recap_progress` 进度事件全在 ha-dash（`recap::slash`），此处
/// 只做转发——ha-core 不依赖特征 crate，实现由 `ha_dash::wire()` 注册到
/// [`crate::recap_hooks`]。未装配即 `Err`，见该模块「未装配语义」。
pub async fn handle_recap(args: &str) -> Result<CommandResult, String> {
    crate::recap_hooks::run_slash_recap(args).await
}
