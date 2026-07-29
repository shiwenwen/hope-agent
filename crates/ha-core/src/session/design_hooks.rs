//! 特征 crate 钩子：session ↔ 设计空间（ha-design）边界。
//!
//! kernel 的 session 生命周期在三个点需要设计特征的行为，全部倒转为装配期
//! 注册（未 wire ＝ 特征不存在，各钩子的缺省语义在各自 accessor 注明）：
//! 工作目录解析（Design 线程绑定代码仓库）、会话清理级联（durable Artifact
//! purge/detach）、incognito 开启守卫（存在 durable Artifact 即拒）。
//!
//! **缺失接线语义**：incognito 开启守卫 **fail-closed**（未 wire 直接拒绝
//! 开启——无法验证 durable Artifact 即不放行，incognito 红线）；清理级联
//! 0 条 + `app_warn` 审计信号（后台 best-effort 路径不 fail 整个清理，但
//! 缺口必须可见）。生产四入口全部 wire，缺失只可能来自新增二进制漏调
//! `ha_design::wire()`。

/// ha-design 装配期注册的 session 侧回调集合（原子注册，单 OnceLock）。
pub struct DesignSessionHooks {
    /// Design 线程的实时工作目录派生（原 `design::service::session_bound_code_dir`）。
    pub bound_code_dir: fn(&str) -> Option<String>,
    /// 会话清理级联：purge（true）或 detach（false）该会话的 durable
    /// Artifact，返回处理条数（原 `ArtifactService::{purge_for_session,
    /// detach_from_session}`；在 blocking 上下文调用，允许同步 IO）。
    pub cleanup_artifacts: fn(&str, bool) -> anyhow::Result<usize>,
    /// incognito 开启守卫：会话是否持有 durable Artifact（原
    /// `ArtifactService::has_for_session`）。
    pub has_durable_artifacts: fn(&str) -> anyhow::Result<bool>,
}

static DESIGN_SESSION_HOOKS: std::sync::OnceLock<DesignSessionHooks> = std::sync::OnceLock::new();

/// 特征 crate 装配期注册。重复注册返回 `Err`。
pub fn register_design_session_hooks(
    hooks: DesignSessionHooks,
) -> Result<(), crate::AlreadyRegistered> {
    DESIGN_SESSION_HOOKS
        .set(hooks)
        .map_err(|_| crate::AlreadyRegistered("design session hooks"))
}

pub(crate) fn design_session_hooks() -> Option<&'static DesignSessionHooks> {
    DESIGN_SESSION_HOOKS.get()
}

/// 测试桩：ha-core 自身测试无法依赖 ha-design（循环依赖），需要走 incognito
/// **成功开启**路径的测试用本桩满足 fail-closed 守卫（语义＝无 durable
/// Artifact）。当前测试都停在更早的 kernel 守卫故暂无消费者——留作
/// fail-closed 的文档化逃生口，新测试直接调用。
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn ensure_test_stub() {
    fn no_dir(_: &str) -> Option<String> {
        None
    }
    fn no_cleanup(_: &str, _: bool) -> anyhow::Result<usize> {
        Ok(0)
    }
    fn no_artifacts(_: &str) -> anyhow::Result<bool> {
        Ok(false)
    }
    let _ = register_design_session_hooks(DesignSessionHooks {
        bound_code_dir: no_dir,
        cleanup_artifacts: no_cleanup,
        has_durable_artifacts: no_artifacts,
    });
}
