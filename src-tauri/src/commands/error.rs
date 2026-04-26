use serde::{Serialize, Serializer};

/// 统一的 Tauri 命令错误类型。
///
/// `#[tauri::command]` 要求返回值实现 `Serialize`，但 `anyhow::Error` 不实现，
/// 历史上每条命令都用 `.map_err(|e| e.to_string())?` 把错误降成 `String`。
/// `CmdError` 在 IPC wire 上等价于一个普通字符串（前端零迁移），同时通过
/// `impl<E: Into<anyhow::Error>> From<E>` 让命令体内部直接用 `?` 透传 `anyhow::Error`、
/// `std::io::Error`、`serde_json::Error` 等任何实现了 `std::error::Error` 的错误。
pub struct CmdError(String);

impl CmdError {
    /// 构造一个纯文本错误（取代散落在各处的 `Err("msg".to_string())` 模式）。
    pub fn msg(m: impl Into<String>) -> Self {
        Self(m.into())
    }
}

impl<E> From<E> for CmdError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        // 用 alternate Display 输出 cause chain，让 `.context("...")?` 加的上下文不丢。
        Self(format!("{:#}", err.into()))
    }
}

impl Serialize for CmdError {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.0)
    }
}
