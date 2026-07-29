//! 后端 JSX/TSX 编译（oxc，纯 Rust、进程内、零外部二进制、零网络）。
//!
//! 把 LLM 产出的交互式组件源（JSX/TSX，classic runtime = `React.createElement`）编译成浏览器
//! 可直接执行的 JS。**编译只在 ha-core 后端发生**，iframe 只加载已编译落盘的静态产物——守
//! 设计空间红线「浏览器零编译/零打包/零 JIT」，同时拿到 Claude Artifacts 级真交互而不重蹈
//! atelier 白屏（编译一次、静态载入、秒开）。
//!
//! 编译失败（畸形/截断源）返回 `Err`，由上层 `degrade_to_placeholder` 降级——**绝不白屏/panic**。

use anyhow::{bail, Result};
use std::path::Path;

use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_transformer::{JsxRuntime, TransformOptions, Transformer};

/// 编译 JSX/TSX 源 → 浏览器可执行 JS（classic runtime，引用全局 `React`）。
/// 只做 JSX 转换 + TS 类型剥离，**不降级现代语法**（目标现代浏览器，hooks/async 原样保留，
/// 无需 helper/bundler）。
pub fn compile_component(source: &str) -> Result<String> {
    if source.trim().is_empty() {
        bail!("component source is empty");
    }
    let allocator = Allocator::default();
    // .tsx：TS 类型剥离 + JSX 转换都开。
    let source_type = SourceType::from_path(Path::new("component.tsx"))
        .map_err(|_| anyhow::anyhow!("invalid source type"))?;

    let parser_ret = Parser::new(&allocator, source, source_type).parse();
    if parser_ret.panicked || !parser_ret.diagnostics.is_empty() {
        let msg = parser_ret
            .diagnostics
            .iter()
            .take(3)
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        bail!("component parse error: {msg}");
    }
    let mut program = parser_ret.program;

    // Semantic 诊断多为提示（未用变量等），不硬失败；只取 scoping 供 transformer。
    let semantic_ret = SemanticBuilder::new().build(&program);
    let scoping = semantic_ret.semantic.into_scoping();

    // classic runtime → `React.createElement` / `React.Fragment`（引用内联的全局 React UMD，
    // 无需 jsx-runtime import，贴「零 bundler」）。默认 env 为空 = 不降级语法。
    let mut options = TransformOptions::default();
    options.jsx.runtime = JsxRuntime::Classic;

    let transform_ret = Transformer::new(&allocator, Path::new("component.tsx"), &options)
        .build_with_scoping(scoping, &mut program);
    if !transform_ret.diagnostics.is_empty() {
        let msg = transform_ret
            .diagnostics
            .iter()
            .take(3)
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        bail!("component transform error: {msg}");
    }

    let code = Codegen::new().build(&program).code;
    if code.trim().is_empty() {
        bail!("component compiled to empty output");
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_jsx_to_create_element() {
        let src = "function App() { return <div className=\"x\">hi</div>; }";
        let out = compile_component(src).expect("compile");
        assert!(out.contains("React.createElement"), "got: {out}");
        assert!(!out.contains("<div"), "JSX not transformed: {out}");
    }

    #[test]
    fn strips_typescript_types() {
        let src = "const n: number = 1; function f(x: string): string { return x; }";
        let out = compile_component(src).expect("compile");
        assert!(!out.contains(": number"), "TS types not stripped: {out}");
        assert!(!out.contains(": string"), "TS types not stripped: {out}");
    }

    #[test]
    fn keeps_hooks_and_modern_syntax() {
        let src = "function App() { const [n, setN] = React.useState(0); return <button onClick={() => setN(n + 1)}>{n}</button>; }";
        let out = compile_component(src).expect("compile");
        assert!(out.contains("useState"), "hooks lost: {out}");
        assert!(
            out.contains("React.createElement"),
            "JSX not transformed: {out}"
        );
    }

    #[test]
    fn empty_source_errors() {
        assert!(compile_component("   ").is_err());
    }

    #[test]
    fn malformed_jsx_errors_not_panics() {
        // Unclosed tag — must return Err, never panic (degrade path relies on this).
        let _ = compile_component("function App() { return <div>oops }");
    }
}
