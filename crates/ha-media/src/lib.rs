//! 媒体特征 crate（阶段 4 第五刀，自 ha-core 迁出）：图/音生成执行机器
//! （23 个 provider adapters / execute / probe / catalog / voices）、STT
//! 执行机器（9 个 provider 协议 / 流式会话 / failover 转写）、
//! `image_generate`·`audio_generate` 两工具。
//!
//! kernel 侧留存：`ha_core::media_gen`（crud / resolve / wire 类型）与
//! `ha_core::stt`（crud / 链解析 / 类型 / errors / 本地 catalog /
//! 转写 trampoline）。ha-design 的图/音 artifact 表单直接依赖本 crate
//! （特征间单向依赖，与 ha-design→ha-browser 同型）——
//! `execute_image`/`execute_audio` 仍是图/音生成唯一入口。
//!
//! 装配契约与其它特征 crate 相同：每个调 `ha_core::init_runtime` 的二进
//! 制必须先调 [`wire()`]。

// `app_*!` 系宏由 ha-base 导出（与 ha-core 同一接法）。
#[macro_use]
extern crate ha_base;

pub mod audio_generate;
pub mod image_generate;
pub mod media_gen;
pub mod stt;

/// 幂等装配：注册两工具分发条目 + STT 转写钩子 + 流式会话 GC 启动任务
/// （PrimaryOnly，与迁移前 app_init 位置的档位一致）。
pub fn wire() {
    static WIRED: std::sync::Once = std::sync::Once::new();
    WIRED.call_once(|| {
        fn image_handler<'a>(
            args: &'a serde_json::Value,
            ctx: &'a ha_core::tools::ToolExecContext,
        ) -> ha_core::tools::registry::BuiltinToolFuture<'a> {
            Box::pin(image_generate::tool_image_generate(args, ctx))
        }
        fn audio_handler<'a>(
            args: &'a serde_json::Value,
            ctx: &'a ha_core::tools::ToolExecContext,
        ) -> ha_core::tools::registry::BuiltinToolFuture<'a> {
            Box::pin(audio_generate::tool_audio_generate(args, ctx))
        }
        ha_core::tools::registry::register_external_tools(vec![
            ha_core::tools::registry::BuiltinToolEntry {
                name: ha_core::tools::TOOL_IMAGE_GENERATE,
                aliases: &[],
                handler: image_handler,
            },
            ha_core::tools::registry::BuiltinToolEntry {
                name: ha_core::tools::TOOL_AUDIO_GENERATE,
                aliases: &[],
                handler: audio_handler,
            },
        ])
        .expect("ha_media::wire() must run before ha_core::init_runtime freezes the tool registry");

        fn transcriber(
            primary: Option<ha_core::stt::ActiveSttModel>,
            fallback: Vec<ha_core::stt::ActiveSttModel>,
            audio: ha_core::stt::AudioPayload,
            options: ha_core::stt::TranscriptOptions,
            session_id: Option<String>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<ha_core::stt::Transcript, ha_core::stt::FailoverError>,
                    > + Send
                    + 'static,
            >,
        > {
            Box::pin(async move {
                stt::engine::failover_transcribe_batch(
                    primary,
                    fallback,
                    audio,
                    &options,
                    session_id.as_deref(),
                )
                .await
            })
        }
        ha_core::stt::register_stt_transcriber(transcriber)
            .expect("ha_media::wire() registers the stt transcriber exactly once");

        // STT 流式会话 GC（每 5 分钟清幽灵会话）。迁移前位于 app_init
        // start_background_tasks 的 primary 块内，档位保持 PrimaryOnly。
        ha_core::app_init::register_startup_task(
            ha_core::app_init::StartupStage::PrimaryOnly,
            || {
                tokio::spawn(async {
                    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(300));
                    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    ticker.tick().await;
                    loop {
                        ticker.tick().await;
                        let evicted = crate::stt::SttSessionManager::global().gc_idle();
                        if evicted > 0 {
                            app_info!(
                                "stt",
                                "session-gc",
                                "evicted {} idle STT session(s)",
                                evicted
                            );
                        }
                    }
                });
            },
        )
        .expect("ha_media::wire() must run before ha_core::init_runtime consumes startup tasks");
    });
}
