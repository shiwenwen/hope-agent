#[cfg(unix)]
use std::process::Command;
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

#[cfg(unix)]
use ha_core::session::{ChatTurnStatus, CommitInterruptedTurn, NewMessage};
use ha_core::session::{CreateStreamRun, JournalBatch, JournalEvent, SessionDB};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct ChildState {
    session_id: String,
    run_id: String,
    ack_seq: u64,
}

#[test]
#[ignore = "subprocess fixture; invoked by crash recovery test"]
fn stream_crash_fixture_child() {
    let Some(root) = std::env::var_os("HA_STREAM_CRASH_FIXTURE_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let db = SessionDB::open(&root.join("sessions.db")).expect("child db");
    let session = db
        .create_session(ha_core::agent_loader::DEFAULT_AGENT_ID)
        .expect("child session");
    let run_id = uuid::Uuid::new_v4().to_string();
    let registration = db
        .create_stream_run(&CreateStreamRun {
            run_id: run_id.clone(),
            session_id: session.id.clone(),
            source: "desktop".to_string(),
            stream_id: Some("crash-stream".to_string()),
            turn_id: None,
            provider_shape: Some("anthropic".to_string()),
        })
        .expect("child run");
    assert!(registration.persistent);
    db.begin_stream_attempt(
        &run_id,
        1,
        Some("fixture"),
        Some("fixture"),
        Some("anthropic"),
    )
    .expect("child attempt");

    for seq in 1..=200u64 {
        db.append_stream_journal_batch(&JournalBatch {
            run_id: run_id.clone(),
            attempt_no: 1,
            block_no: seq,
            seq_start: seq,
            seq_end: seq,
            events: vec![JournalEvent {
                seq_start: None,
                seq,
                event: serde_json::json!({
                    "type":"text_delta",
                    "content":format!("{seq}|")
                })
                .to_string(),
            }],
        })
        .expect("child journal append");

        // This ack stands in for the post-durability UI broadcast: it is
        // written only after SQLite's FULL-synchronous transaction returned.
        std::fs::write(
            root.join("state.json"),
            serde_json::to_vec(&ChildState {
                session_id: session.id.clone(),
                run_id: run_id.clone(),
                ack_seq: seq,
            })
            .expect("state json"),
        )
        .expect("state write");
        std::thread::sleep(Duration::from_millis(8));
    }
    std::thread::sleep(Duration::from_secs(30));
}

#[test]
#[cfg(unix)]
fn kill_9_recovers_every_acknowledged_delta_as_one_continuous_prefix() {
    let dir = tempfile::tempdir().expect("tempdir");
    let exe = std::env::current_exe().expect("current test executable");
    let mut child = Command::new(exe)
        .arg("--exact")
        .arg("stream_crash_fixture_child")
        .arg("--ignored")
        .arg("--nocapture")
        .env("HA_STREAM_CRASH_FIXTURE_ROOT", dir.path())
        .spawn()
        .expect("spawn crash fixture");

    let deadline = Instant::now() + Duration::from_secs(10);
    let acknowledged = loop {
        if let Ok(bytes) = std::fs::read(dir.path().join("state.json")) {
            if let Ok(state) = serde_json::from_slice::<ChildState>(&bytes) {
                if state.ack_seq >= 25 {
                    break state;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "fixture did not reach durable prefix"
        );
        std::thread::sleep(Duration::from_millis(10));
    };

    child.kill().expect("SIGKILL fixture");
    let _ = child.wait().expect("wait fixture");

    let db = SessionDB::open(&dir.path().join("sessions.db")).expect("reopen after kill");
    let snapshot = db
        .stream_run_snapshot(&acknowledged.run_id)
        .expect("snapshot")
        .expect("unfinished run");
    assert_eq!(snapshot.run.status, "running");

    let mut previous = 0u64;
    let mut recovered_text = String::new();
    for block in &snapshot.journal {
        assert_eq!(block.seq_start, previous + 1, "journal gap after kill");
        assert!(ha_core::session::verify_block(block), "checksum after kill");
        let events: Vec<JournalEvent> = serde_json::from_str(&block.payload).expect("events");
        for event in events {
            assert_eq!(event.seq, previous + 1);
            previous = event.seq;
            let payload: serde_json::Value = serde_json::from_str(&event.event).expect("event");
            recovered_text.push_str(
                payload
                    .get("content")
                    .and_then(|value| value.as_str())
                    .expect("content"),
            );
        }
    }
    assert!(previous >= acknowledged.ack_seq);
    for seq in 1..=acknowledged.ack_seq {
        assert!(recovered_text.contains(&format!("{seq}|")));
    }

    let (_, context_revision) = db
        .load_context_with_revision(&acknowledged.session_id)
        .expect("context revision");
    let commit = CommitInterruptedTurn {
        run_id: Some(acknowledged.run_id.clone()),
        attempt_no: 1,
        session_id: acknowledged.session_id.clone(),
        assistant: Some(NewMessage::assistant(&recovered_text)),
        context_json: "[]".to_string(),
        expected_context_revision: context_revision,
        turn_id: None,
        final_seq: previous,
        status: ChatTurnStatus::Interrupted,
        interrupt_reason: Some("crash_recovery".to_string()),
        error: None,
        recovery_event: Some(NewMessage::error_event("Recovered after crash")),
    };
    let first = db.commit_interrupted_turn(&commit).expect("recover run");
    let second = db
        .commit_interrupted_turn(&commit)
        .expect("idempotent recovery replay");
    assert_eq!(first.assistant_message_id, second.assistant_message_id);
    let recovered = db
        .stream_run_snapshot(&acknowledged.run_id)
        .expect("recovered snapshot")
        .expect("recovered run");
    assert_eq!(recovered.run.status, "recovered");
    assert_eq!(recovered.run.committed_seq, previous);
}
