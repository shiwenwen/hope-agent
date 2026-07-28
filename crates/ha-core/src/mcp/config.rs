//! MCP server configuration schema.
//!
//! 类型定义（`McpServerConfig` / `McpGlobalSettings` / `McpTransportSpec` /
//! `McpOAuthConfig` / `McpTrustLevel`）已下沉 [`ha_config_schema::mcp`]，此处
//! 原地再导出保持 `crate::mcp::config::*` 路径不变。留在本文件的是子系统逻辑：
//! 保存期校验 `validate_server_config`（返回 `mcp::errors` 错误类型）、名称
//! 校验、env 占位符展开，以及它们的测试。

// 类型已下沉 ha-config-schema
pub use ha_config_schema::mcp::{
    McpGlobalSettings, McpOAuthConfig, McpServerConfig, McpTransportSpec, McpTrustLevel,
};

/// Name regex: lowercase letters, digits, underscore, hyphen; 1–32 chars.
/// Hand-rolled to avoid pulling a regex just for one check at save time.
pub fn is_valid_name(s: &str) -> bool {
    let len = s.len();
    if !(1..=32).contains(&len) {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

// ── Env Placeholder Expansion ────────────────────────────────────

/// Expand `${VAR}` / `$VAR` placeholders in a value string using a lookup.
///
/// Rules (kept narrow on purpose):
/// * `${VAR}` — braced form, always honored.
/// * `$VAR` — unbraced, honored when followed by alphanumerics/underscore.
/// * An unknown variable resolves to the empty string. Callers can detect
///   this by comparing pre/post or by pre-validating keys.
/// * `$$` is an escape for a literal `$`.
///
/// We don't use `std::env::var()` directly — callers pass their own
/// lookup so project-scoped env blocks can override without touching
/// the process environment.
pub fn expand_placeholders<F>(input: &str, mut lookup: F) -> String
where
    F: FnMut(&str) -> Option<String>,
{
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b != b'$' {
            out.push(b as char);
            i += 1;
            continue;
        }
        // `$$` → literal `$`
        if i + 1 < bytes.len() && bytes[i + 1] == b'$' {
            out.push('$');
            i += 2;
            continue;
        }
        // `${...}`
        if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(end) = bytes[i + 2..].iter().position(|&c| c == b'}') {
                let name = &input[i + 2..i + 2 + end];
                if let Some(v) = lookup(name) {
                    out.push_str(&v);
                }
                i += 2 + end + 1;
                continue;
            }
        }
        // `$VAR` (bare)
        let name_start = i + 1;
        let mut name_end = name_start;
        while name_end < bytes.len() {
            let c = bytes[name_end];
            if c.is_ascii_alphanumeric() || c == b'_' {
                name_end += 1;
            } else {
                break;
            }
        }
        if name_end > name_start {
            let name = &input[name_start..name_end];
            if let Some(v) = lookup(name) {
                out.push_str(&v);
            }
            i = name_end;
        } else {
            out.push('$');
            i += 1;
        }
    }
    out
}

// ── Tests ────────────────────────────────────────────────────────

/// 校验一条 MCP server 配置，任何不变量违规返回 `Err(McpError::Config(..))`。
/// 设置面板 / 导入路径在保存时调用；`McpManager::init` 对每条记录再防御性跑一遍，
/// 以便隔离 legacy 数据。
///
/// 原为 `McpServerConfig` 的固有方法，因返回 `mcp::errors` 的子系统错误类型而
/// 移出——类型已下沉 `ha-config-schema`，schema 层不得依赖子系统。
pub fn validate_server_config(cfg: &McpServerConfig) -> crate::mcp::errors::McpResult<()> {
    use crate::mcp::errors::McpError;
    if !is_valid_name(&cfg.name) {
        return Err(McpError::Config(format!(
            "invalid server name '{}': must match ^[a-z0-9_-]{{1,32}}$",
            cfg.name
        )));
    }
    if cfg.id.is_empty() {
        return Err(McpError::Config("server id must not be empty".into()));
    }
    match &cfg.transport {
        McpTransportSpec::Stdio { command, .. } if command.trim().is_empty() => {
            return Err(McpError::Config(format!(
                "server '{}': stdio command must not be empty",
                cfg.name
            )));
        }
        McpTransportSpec::StreamableHttp { url }
        | McpTransportSpec::Sse { url }
        | McpTransportSpec::WebSocket { url }
            if url.trim().is_empty() =>
        {
            return Err(McpError::Config(format!(
                "server '{}': transport URL must not be empty",
                cfg.name
            )));
        }
        _ => {}
    }
    if cfg.auto_approve && matches!(cfg.trust_level, McpTrustLevel::Untrusted) {
        return Err(McpError::Config(format!(
            "server '{}': auto_approve requires trust_level=trusted",
            cfg.name
        )));
    }
    if cfg.oauth.is_some() && !cfg.transport.is_networked() {
        return Err(McpError::Config(format!(
            "server '{}': OAuth is only supported on networked transports",
            cfg.name
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_regex_accepts_valid() {
        assert!(is_valid_name("a"));
        assert!(is_valid_name("my-server_01"));
        assert!(is_valid_name(&"a".repeat(32)));
    }

    #[test]
    fn name_regex_rejects_invalid() {
        assert!(!is_valid_name(""));
        assert!(!is_valid_name(&"a".repeat(33)));
        assert!(!is_valid_name("Foo")); // uppercase
        assert!(!is_valid_name("with space"));
        assert!(!is_valid_name("dot.separator"));
    }

    #[test]
    fn validate_rejects_auto_approve_on_untrusted() {
        let cfg = McpServerConfig {
            id: "id-1".into(),
            name: "foo".into(),
            enabled: true,
            transport: McpTransportSpec::Stdio {
                command: "true".into(),
                args: vec![],
                cwd: None,
            },
            env: Default::default(),
            headers: Default::default(),
            oauth: None,
            allowed_tools: vec![],
            denied_tools: vec![],
            connect_timeout_secs: 30,
            call_timeout_secs: 120,
            health_check_interval_secs: 60,
            max_concurrent_calls: 4,
            auto_approve: true, // conflict
            trust_level: McpTrustLevel::Untrusted,
            eager: false,
            deferred_tools: false,
            project_paths: vec![],
            description: None,
            icon: None,
            created_at: 0,
            updated_at: 0,
            trust_acknowledged_at: None,
        };
        assert!(crate::mcp::config::validate_server_config(&cfg).is_err());
    }

    #[test]
    fn validate_accepts_minimal_stdio() {
        let cfg = McpServerConfig {
            id: "id-1".into(),
            name: "foo".into(),
            enabled: true,
            transport: McpTransportSpec::Stdio {
                command: "true".into(),
                args: vec![],
                cwd: None,
            },
            env: Default::default(),
            headers: Default::default(),
            oauth: None,
            allowed_tools: vec![],
            denied_tools: vec![],
            connect_timeout_secs: 30,
            call_timeout_secs: 120,
            health_check_interval_secs: 60,
            max_concurrent_calls: 4,
            auto_approve: false,
            trust_level: McpTrustLevel::Untrusted,
            eager: false,
            deferred_tools: false,
            project_paths: vec![],
            description: None,
            icon: None,
            created_at: 0,
            updated_at: 0,
            trust_acknowledged_at: None,
        };
        assert!(crate::mcp::config::validate_server_config(&cfg).is_ok());
    }

    #[test]
    fn validate_rejects_stdio_oauth() {
        let cfg = McpServerConfig {
            id: "id-1".into(),
            name: "foo".into(),
            enabled: true,
            transport: McpTransportSpec::Stdio {
                command: "true".into(),
                args: vec![],
                cwd: None,
            },
            env: Default::default(),
            headers: Default::default(),
            oauth: Some(McpOAuthConfig {
                client_id: Some("client".into()),
                client_secret: None,
                authorization_endpoint: None,
                token_endpoint: None,
                scopes: vec![],
                extra_params: Default::default(),
            }),
            allowed_tools: vec![],
            denied_tools: vec![],
            connect_timeout_secs: 30,
            call_timeout_secs: 120,
            health_check_interval_secs: 60,
            max_concurrent_calls: 4,
            auto_approve: false,
            trust_level: McpTrustLevel::Untrusted,
            eager: false,
            deferred_tools: false,
            project_paths: vec![],
            description: None,
            icon: None,
            created_at: 0,
            updated_at: 0,
            trust_acknowledged_at: None,
        };

        assert!(crate::mcp::config::validate_server_config(&cfg).is_err());
    }

    #[test]
    fn validate_rejects_empty_command() {
        let cfg = McpServerConfig {
            id: "id-1".into(),
            name: "foo".into(),
            enabled: true,
            transport: McpTransportSpec::Stdio {
                command: "  ".into(),
                args: vec![],
                cwd: None,
            },
            env: Default::default(),
            headers: Default::default(),
            oauth: None,
            allowed_tools: vec![],
            denied_tools: vec![],
            connect_timeout_secs: 30,
            call_timeout_secs: 120,
            health_check_interval_secs: 60,
            max_concurrent_calls: 4,
            auto_approve: false,
            trust_level: McpTrustLevel::Untrusted,
            eager: false,
            deferred_tools: false,
            project_paths: vec![],
            description: None,
            icon: None,
            created_at: 0,
            updated_at: 0,
            trust_acknowledged_at: None,
        };
        assert!(crate::mcp::config::validate_server_config(&cfg).is_err());
    }

    #[test]
    fn expand_braced_placeholders() {
        let s = expand_placeholders("${FOO}/bar/${BAZ}", |k| match k {
            "FOO" => Some("x".into()),
            "BAZ" => Some("y".into()),
            _ => None,
        });
        assert_eq!(s, "x/bar/y");
    }

    #[test]
    fn expand_bare_placeholders() {
        let s = expand_placeholders("$HOME/hope", |k| {
            if k == "HOME" {
                Some("/Users/test".into())
            } else {
                None
            }
        });
        assert_eq!(s, "/Users/test/hope");
    }

    #[test]
    fn expand_escaped_dollar() {
        let s = expand_placeholders("price: $$5 (${X})", |k| {
            if k == "X" {
                Some("five".into())
            } else {
                None
            }
        });
        assert_eq!(s, "price: $5 (five)");
    }

    #[test]
    fn expand_unknown_vars_become_empty() {
        let s = expand_placeholders("hi ${UNDEF}!", |_| None);
        assert_eq!(s, "hi !");
    }

    #[test]
    fn transport_kind_labels() {
        assert_eq!(
            McpTransportSpec::Stdio {
                command: "x".into(),
                args: vec![],
                cwd: None,
            }
            .kind_label(),
            "stdio"
        );
        assert_eq!(
            McpTransportSpec::StreamableHttp {
                url: "https://x".into()
            }
            .kind_label(),
            "http"
        );
        assert_eq!(
            McpTransportSpec::Sse {
                url: "https://x".into()
            }
            .kind_label(),
            "sse"
        );
        assert_eq!(
            McpTransportSpec::WebSocket {
                url: "wss://x".into()
            }
            .kind_label(),
            "ws"
        );
    }

    #[test]
    fn global_settings_default_enabled() {
        let g = McpGlobalSettings::default();
        assert!(g.enabled);
        assert_eq!(g.max_concurrent_calls, 8);
        assert_eq!(g.backoff_initial_secs, 5);
        assert_eq!(g.backoff_max_secs, 300);
    }

    #[test]
    fn deserialize_transport_variants() {
        let stdio: McpTransportSpec =
            serde_json::from_str(r#"{"kind":"stdio","command":"foo","args":["-x"]}"#).unwrap();
        assert!(matches!(stdio, McpTransportSpec::Stdio { .. }));
        let http: McpTransportSpec =
            serde_json::from_str(r#"{"kind":"streamableHttp","url":"https://example.com/mcp"}"#)
                .unwrap();
        assert!(matches!(http, McpTransportSpec::StreamableHttp { .. }));
    }
}
