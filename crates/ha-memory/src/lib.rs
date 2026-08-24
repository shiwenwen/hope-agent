//! Memory execution machines wired above `ha-core`.

#[macro_use]
extern crate ha_core;

pub mod dreaming_context_pack;
pub mod dreaming_cron_loop;
pub mod dreaming_evidence;
pub mod dreaming_narrative;
pub mod dreaming_pipeline;
pub mod dreaming_profile;
pub mod dreaming_promotion;
pub mod dreaming_resolver;
pub mod dreaming_scanner;
pub mod dreaming_scoring;
pub mod dreaming_triggers;
pub mod embedding;
pub mod external_provider;
pub mod extract;
pub mod recall_planner;
pub mod recall_summary;
pub mod reembed;

/// Register all memory runtime ports before `ha_core::init_runtime` freezes
/// assembly. Repeated calls from shared shell wiring are safe.
pub fn wire() {
    static WIRED: std::sync::Once = std::sync::Once::new();
    WIRED.call_once(|| {
        ha_core::memory::dreaming::register_context_pack_runtime(
            ha_core::memory::dreaming::ContextPackRuntime {
                build: dreaming_context_pack::build_context_pack,
            },
        )
        .expect("ha-memory Context Pack runtime must be registered exactly once");
        ha_core::memory::dreaming::register_dreaming_evidence_runtime(
            ha_core::memory::dreaming::DreamingEvidenceRuntime {
                evidence_quote: dreaming_evidence::evidence_quote,
            },
        )
        .expect("ha-memory Dreaming evidence runtime must be registered exactly once");
        ha_core::memory::dreaming::register_dreaming_promotion_runtime(
            ha_core::memory::dreaming::DreamingPromotionRuntime {
                apply: dreaming_promotion::apply_promotions,
            },
        )
        .expect("ha-memory Dreaming promotion runtime must be registered exactly once");
        ha_core::memory::dreaming::register_dreaming_pipeline_runtime(
            ha_core::memory::dreaming::DreamingPipelineRuntime {
                last_report_snapshot: dreaming_pipeline::last_report_snapshot,
                run_cycle: dreaming_pipeline::run_cycle_boxed,
            },
        )
        .expect("ha-memory Dreaming pipeline runtime must be registered exactly once");
        ha_core::memory::dreaming::register_dreaming_profile_runtime(
            ha_core::memory::dreaming::DreamingProfileRuntime {
                run_cycle: dreaming_profile::run_profile_synthesis_cycle_boxed,
            },
        )
        .expect("ha-memory Dreaming profile runtime must be registered exactly once");
        ha_core::memory::dreaming::register_dreaming_trigger_runtime(
            ha_core::memory::dreaming::DreamingTriggerRuntime {
                dreaming_running: dreaming_triggers::dreaming_running,
                last_activity_epoch_secs: dreaming_triggers::last_activity_epoch_secs,
                touch_activity: dreaming_triggers::touch_activity,
                check_idle_trigger: dreaming_triggers::check_idle_trigger,
                manual_run: dreaming_triggers::manual_run_boxed,
                spawn_cron_loop: dreaming_cron_loop::spawn_dreaming_cron_loop,
            },
        )
        .expect("ha-memory Dreaming trigger runtime must be registered exactly once");
        ha_core::memory::dreaming::register_dreaming_resolver_runtime(
            ha_core::memory::dreaming::DreamingResolverRuntime {
                preflight: dreaming_resolver::resolver_preflight,
                preflight_from_claims: dreaming_resolver::resolver_preflight_from_claims,
                run_cycle: dreaming_resolver::run_resolver_cycle_boxed,
                plan_auto_expiration: dreaming_resolver::plan_auto_expiration_sweep,
                plan_auto_groups: dreaming_resolver::plan_auto_resolution_groups,
            },
        )
        .expect("ha-memory Deep Resolver runtime must be registered exactly once");
        ha_core::memory::embedding::factory::register_embedding_factory(
            embedding::create_embedding_provider,
        )
        .expect("ha-memory embedding factory must be registered exactly once");
        ha_core::memory::extract_runtime::register(
            ha_core::memory::extract_runtime::MemoryExtractRuntime {
                run_extraction: extract::run_extraction_boxed,
                flush_before_compact: extract::flush_before_compact_boxed,
                spawn_tracked_extraction: extract::spawn_tracked_extraction,
                cancel_active_extractions: extract::cancel_active_extractions,
                cancel_idle_extraction: extract::cancel_idle_extraction,
                schedule_idle_extraction: extract::schedule_idle_extraction,
                flush_all_idle_extractions: extract::flush_all_idle_extractions,
            },
        )
        .expect("ha-memory extraction runtime must be registered exactly once");
        ha_core::memory::reembed_job::register_reembed_runtime(
            ha_core::memory::reembed_job::MemoryReembedRuntime {
                cancel_active: reembed::cancel_active_memory_reembed_jobs,
                start: reembed::start_memory_reembed_job,
            },
        )
        .expect("ha-memory reembed runtime must be registered exactly once");
        ha_core::memory::recall_summary::register_recall_summary_runtime(
            ha_core::memory::recall_summary::RecallSummaryRuntime {
                summarize: recall_summary::summarize_boxed,
            },
        )
        .expect("ha-memory recall summary runtime must be registered exactly once");
        ha_core::memory::external_provider::register_external_memory_runtime(
            ha_core::memory::external_provider::ExternalMemoryRuntime {
                execute_sync: external_provider::execute_sync_boxed,
                schedule_sync: external_provider::schedule_external_memory_provider_sync,
                spawn_sync_loop: external_provider::spawn_external_memory_provider_sync_loop,
                hydrate_config: external_provider::hydrate_external_memory_provider_config,
                save_credentials: external_provider::save_credentials_boxed,
                test_connection: external_provider::test_connection_boxed,
                compatibility_snapshot:
                    external_provider::external_memory_provider_compatibility_snapshot,
                credential_status:
                    external_provider::get_external_memory_provider_credential_status,
                clear_credentials: external_provider::clear_external_memory_provider_credentials,
                save_config: external_provider::save_external_memory_providers_config,
                patch_config: external_provider::patch_external_memory_providers_config,
            },
        )
        .expect("ha-memory external provider runtime must be registered exactly once");
        ha_core::memory::recall_planner::register_memory_retrieval_runtime(
            ha_core::memory::recall_planner::MemoryRetrievalRuntime {
                plan_fast: recall_planner::plan_fast_recall,
                evidence_relevant: recall_planner::retrieval_evidence_is_relevant,
                build_deep: recall_planner::build_deep_recall_prompt,
                parse_deep: recall_planner::parse_deep_recall_response,
                apply_deep: recall_planner::apply_deep_recall,
            },
        )
        .expect("ha-memory retrieval runtime must be registered exactly once");
        ha_core::memory::dreaming::scoring::register_dreaming_scoring_runtime(
            ha_core::memory::dreaming::scoring::DreamingScoringRuntime {
                parse_nominations: dreaming_scoring::parse_nominations,
                filter_and_rank: dreaming_scoring::filter_and_rank,
            },
        )
        .expect("ha-memory Dreaming scoring runtime must be registered exactly once");
        ha_core::memory::dreaming::scanner::register_dreaming_scanner_runtime(
            ha_core::memory::dreaming::scanner::DreamingScannerRuntime {
                collect_candidates: dreaming_scanner::collect_candidates,
                evidence_for_candidate: dreaming_scanner::evidence_for_candidate,
                render_candidates_for_prompt: dreaming_scanner::render_candidates_for_prompt,
            },
        )
        .expect("ha-memory Dreaming scanner runtime must be registered exactly once");
        ha_core::memory::dreaming::narrative::register_dreaming_narrative_runtime(
            ha_core::memory::dreaming::narrative::DreamingNarrativeRuntime {
                build_prompt: dreaming_narrative::build_prompt,
                run_side_query: dreaming_narrative::run_side_query_boxed,
                render_diary_markdown: dreaming_narrative::render_diary_markdown,
                write_diary: dreaming_narrative::write_diary,
            },
        )
        .expect("ha-memory Dreaming narrative runtime must be registered exactly once");
    });
}
