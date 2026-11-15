//! Deterministic loopback acceptance of the real source → kernel → Provider
//! stream → report tool → committed private finding path. No real credentials.
use ha_core::personal_assistant::{policy::AssistantPolicySettings, research};
use ha_core::provider::{ActiveModel, ApiType, ModelConfig, ProviderConfig};
use ha_core::session::{NewMessage, SessionDB};
use serde_json::json;
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_file_is_reviewed_through_the_real_kernel_and_private_report_tool() {
    let dir = tempfile::tempdir().unwrap();
    ha_core::test_support::with_env_vars_async(&[("HA_DATA_DIR", dir.path())], || async {
        ha_core::config::replace_cache_for_test(Default::default());
        ha_agent_runtime::wire();
        assert_eq!(ha_core::runtime_lock::acquire_or_secondary("assistant-research-test"), ha_core::runtime_lock::Tier::Primary);
        let db=Arc::new(SessionDB::open(&dir.path().join("sessions.db")).unwrap());
        ha_core::channel::db::ChannelDB::new(db.clone()).migrate().unwrap();
        let work=db.create_session("ha-main").unwrap();
        let workspace=dir.path().join("workspace");std::fs::create_dir(&workspace).unwrap();
        db.update_session_working_dir(&work.id,Some(workspace.to_str().unwrap().into())).unwrap();
        let home=db.ensure_personal_assistant_home().unwrap();
        db.append_message(&home.id,&NewMessage::user("PRIVATE_HOME_NOT_SHARED_SENTINEL")).unwrap();
        db.request_assistant_policy(0,AssistantPolicySettings { research_enabled:true,..Default::default() }).unwrap();
        ha_core::personal_assistant::policy::apply_policy(db.clone()).await.unwrap();
        std::fs::write(workspace.join("agenda.txt"),"Planning 09:00\nReview 10:00\n").unwrap();
        research::create_source(db.clone(),research::AssistantSourceInput{
            request_key:uuid::Uuid::new_v4().to_string(),session_id:work.id,relative_path:"agenda.txt".into(),
            focus:"Identify meeting conflicts and prepare alternatives".into(),duration_days:7,
        }).await.unwrap();
        assert!(db.list_assistant_findings(chrono::Utc::now().timestamp()).unwrap().is_empty());
        std::fs::write(workspace.join("agenda.txt"),"Planning 09:00\nReview 09:00\n").unwrap();
        research::refresh_owner_sources(db.clone()).await.unwrap();
        let source=db.list_assistant_sources().unwrap().remove(0);
        let args=json!({"findings":[{"topicKey":"meeting-conflict","fact":"Both meetings are at 09:00",
            "inference":"The owner may be unable to attend both","uncertainty":"Attendance requirements are unknown",
            "prepared":"Option A: move the review. Option B: join one remotely.",
            "references":[{"sourceId":source.id,"revision":source.revision,"version":source.version,"startLine":1,"endLine":2}]}]});
        let server=MockServer::start().await;
        let events=[
            json!({"type":"message_start","message":{"id":"mock-response","type":"message","role":"assistant","content":[],"model":"research-mock","usage":{"input_tokens":10,"output_tokens":0}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"report-1","name":"assistant_research_report","input":{}}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":args.to_string()}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":10}}),
            json!({"type":"message_stop"}),
        ].into_iter().map(|event|format!("event: {}\ndata: {event}\n\n",event["type"].as_str().unwrap())).collect::<String>();
        Mock::given(method("POST")).and(path("/v1/messages")).respond_with(ResponseTemplate::new(200).insert_header("content-type","text/event-stream").set_body_string(events)).mount(&server).await;
        let mut provider=ProviderConfig::new("Isolated research mock".into(),ApiType::Anthropic,server.uri(),"unused-test-key".into());
        provider.id="research-mock".into();
        provider.models.push(ModelConfig {id:"research-mock".into(),name:"Research mock".into(),input_types:vec!["text".into()],context_window:128_000,max_tokens:4096,reasoning:false,thinking_style:None,cost_input:Some(0.0),cost_output:Some(0.0)});
        let mut config=(*ha_core::config::cached_config()).clone();config.providers=vec![provider];config.active_model=Some(ActiveModel{provider_id:"research-mock".into(),model_id:"research-mock".into()});
        ha_core::config::replace_cache_for_test(config);
        research::register_coordinator();
        research::process_pending_research(db.clone()).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(25),async {
            loop {
                let findings=db.list_assistant_findings(chrono::Utc::now().timestamp()).unwrap();
                if !findings.is_empty() {assert!(findings[0].prepared.contains("Option B"));break;}
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }).await.expect("the real report stream commits and publishes a finding");
        let requests=server.received_requests().await.unwrap();
        assert_eq!(requests.len(),1,"one prepared generation, with no remote count or auxiliary request");
        let body:String=String::from_utf8(requests[0].body.clone()).unwrap();
        assert!(body.contains("Review 09:00"));assert!(!body.contains("PRIVATE_HOME_NOT_SHARED_SENTINEL"));
        assert!(body.contains("assistant_research_report"));
        assert!(db.load_session_messages(&home.id).unwrap().iter().all(|message| !message.content.contains("Both meetings are at 09:00")));
        assert!(db.list_assistant_items().unwrap().is_empty(),"discovery does not create a Goal or execution item");
        ha_core::config::replace_cache_for_test(Default::default());
    }).await;
}
