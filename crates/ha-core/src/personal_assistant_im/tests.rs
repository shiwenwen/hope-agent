use super::*;
use crate::session::SessionDB;
use std::path::PathBuf;

const NOW: i64 = 1_800_000_000;
const OWNER: i64 = 42;

fn database() -> (tempfile::TempDir, PathBuf, SessionDB) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sessions.db");
    let db = SessionDB::open(&path).unwrap();
    db.with_conn_for_test(|conn| {
        migrate(conn)?;
        conn.execute(
            "INSERT INTO sessions(id, created_at, updated_at) VALUES ('work', 'now', 'now')",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    (directory, path, db)
}

fn connected(db: &SessionDB) {
    db.save_assistant_im_connection(100, "private_bot", &uuid::Uuid::new_v4().to_string(), NOW)
        .unwrap();
}

fn input(id: i64, kind: TelegramInputKind) -> TelegramInput {
    TelegramInput {
        update_id: id,
        sent_at: Some(NOW),
        user_id: OWNER,
        chat_id: OWNER,
        kind,
    }
}

fn activate(db: &SessionDB, first_event: i64) -> PollLease {
    let code = db.begin_assistant_im_pairing(NOW).unwrap();
    let expected = db.assistant_im_poll_lease(NOW).unwrap().unwrap();
    assert_eq!(
        db.receive_assistant_im_update(
            &expected,
            input(first_event, TelegramInputKind::PairCode(code.code)),
            NOW
        )
        .unwrap(),
        IngressOutcome::PairingCandidate
    );
    let prompt = db
        .confirm_assistant_im_pairing(expected.generation, OWNER, OWNER, NOW)
        .unwrap();
    let secret = prompt
        .callback_data
        .strip_prefix(ACTIVATE_PREFIX)
        .unwrap()
        .to_owned();
    assert_eq!(
        db.receive_assistant_im_update(
            &expected,
            input(
                first_event + 1,
                TelegramInputKind::Message("old pending command".into())
            ),
            NOW
        )
        .unwrap(),
        IngressOutcome::Ignored
    );
    assert_eq!(
        db.receive_assistant_im_update(
            &expected,
            input(first_event + 2, TelegramInputKind::Activate(secret)),
            NOW
        )
        .unwrap(),
        IngressOutcome::Activated
    );
    db.assistant_im_poll_lease(NOW).unwrap().unwrap()
}

fn query_grant(db: &SessionDB) -> SessionGrant {
    db.set_assistant_im_grant(
        SessionGrant {
            session_id: "work".into(),
            revision: 0,
            query_status: true,
            delegate: false,
            stop: false,
            completion_notice: false,
            expires_at: NOW + 3_600,
        },
        NOW,
    )
    .unwrap()
}

#[test]
fn stable_bot_purpose_survives_release_and_competing_connections() {
    let (_directory, path, db) = database();
    db.claim_telegram_bot(100, BotPurpose::Ordinary, "work-account")
        .unwrap();
    db.release_telegram_bot(100, "work-account").unwrap();
    let conflict = db
        .save_assistant_im_connection(100, "private_bot", &uuid::Uuid::new_v4().to_string(), NOW)
        .unwrap_err();
    assert_eq!(
        conflict.downcast_ref::<AssistantImError>(),
        Some(&AssistantImError::IdentityInUse)
    );
    let other = SessionDB::open(&path).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let first_barrier = barrier.clone();
    let first = std::thread::spawn(move || {
        first_barrier.wait();
        db.claim_telegram_bot(200, BotPurpose::Ordinary, "work-2")
    });
    let second = std::thread::spawn(move || {
        barrier.wait();
        other.claim_telegram_bot(200, BotPurpose::PersonalAssistant, "private-2")
    });
    assert_ne!(
        first.join().unwrap().is_ok(),
        second.join().unwrap().is_ok()
    );
}

#[test]
fn pairing_boundary_drops_backlog_and_restarts_do_not_duplicate_commands() {
    let (_directory, path, db) = database();
    connected(&db);
    let expected = activate(&db, 10);
    query_grant(&db);
    let command = || input(13, TelegramInputKind::Message("check work".into()));
    assert_eq!(
        db.receive_assistant_im_update(&expected, command(), NOW)
            .unwrap(),
        IngressOutcome::Queued
    );
    drop(db);
    let db = SessionDB::open(&path).unwrap();
    assert_eq!(
        db.receive_assistant_im_update(&expected, command(), NOW)
            .unwrap(),
        IngressOutcome::Duplicate
    );
    let pending = db.assistant_im_pending_commands().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].grant_revisions.get("work"), Some(&1));
    assert_eq!(db.assistant_im_poll_lease(NOW).unwrap().unwrap().cursor, 14);
    db.revoke_assistant_im_pairing().unwrap();
    assert!(db
        .receive_assistant_im_update(
            &expected,
            input(14, TelegramInputKind::Message("old".into())),
            NOW
        )
        .is_err());
    assert!(db.assistant_im_pending_commands().unwrap().is_empty());
}

#[test]
fn revoke_grant_cancels_reply_and_does_not_reset_pairing_or_reuse_revision() {
    let (_directory, _path, db) = database();
    connected(&db);
    let expected = activate(&db, 10);
    let permission = query_grant(&db);
    db.enqueue_assistant_im_reply(
        &expected,
        "request",
        "work",
        permission.revision,
        false,
        "Work is active",
        NOW,
    )
    .unwrap();
    db.revoke_assistant_im_grant("work").unwrap();
    assert!(db.prepare_assistant_im_delivery(NOW).unwrap().is_none());
    assert_eq!(
        db.assistant_im_connection().unwrap().unwrap().state,
        PairingState::Active
    );
    assert!(db.set_assistant_im_grant(permission, NOW).is_err());
    assert_eq!(
        db.assistant_im_delivery_receipts().unwrap()[0].state,
        DeliveryState::Cancelled
    );
}

#[test]
fn send_crash_is_unknown_and_cannot_be_replayed_by_same_request() {
    let (_directory, path, db) = database();
    connected(&db);
    let expected = activate(&db, 10);
    let permission = query_grant(&db);
    let id = db
        .enqueue_assistant_im_reply(
            &expected,
            "request",
            "work",
            permission.revision,
            false,
            "Work is active",
            NOW,
        )
        .unwrap();
    assert_eq!(
        db.prepare_assistant_im_delivery(NOW).unwrap().unwrap().id,
        id
    );
    drop(db);
    let db = SessionDB::open(&path).unwrap();
    assert_eq!(db.recover_assistant_im_deliveries().unwrap(), 1);
    assert_eq!(
        db.enqueue_assistant_im_reply(
            &expected,
            "request",
            "work",
            permission.revision,
            false,
            "Work is active",
            NOW
        )
        .unwrap(),
        id
    );
    assert!(db.prepare_assistant_im_delivery(NOW).unwrap().is_none());
    assert_eq!(
        db.assistant_im_delivery_receipts().unwrap()[0].state,
        DeliveryState::Unknown
    );
}

#[test]
fn idle_week_resets_cursor_and_unprovable_outage_requires_new_pairing() {
    let (_directory, _path, db) = database();
    connected(&db);
    let expected = activate(&db, 1_000);
    let later = NOW + 7 * 86_400;
    db.assistant_im_poll_completed(&expected, later - 20)
        .unwrap();
    let after_week = db.assistant_im_poll_lease(later).unwrap().unwrap();
    assert_eq!(after_week.cursor, 0);
    assert!(after_week.cursor_epoch > expected.cursor_epoch);
    assert_eq!(
        db.assistant_im_connection().unwrap().unwrap().state,
        PairingState::Active
    );
    let mut fresh = input(10, TelegramInputKind::Message("new lower id".into()));
    fresh.sent_at = Some(later);
    assert_eq!(
        db.receive_assistant_im_update(&after_week, fresh, later)
            .unwrap(),
        IngressOutcome::Queued
    );
    let after_gap = db
        .assistant_im_poll_lease(later + 7 * 86_400)
        .unwrap()
        .unwrap();
    assert!(after_gap.generation > after_week.generation);
    assert_eq!(
        db.assistant_im_connection().unwrap().unwrap().state,
        PairingState::Unpaired
    );
    assert!(db.assistant_im_pending_commands().unwrap().is_empty());
}

#[test]
fn restart_consumes_stale_or_unprovable_messages_without_executing_them() {
    let (_directory, path, db) = database();
    connected(&db);
    activate(&db, 100);
    drop(db);
    let db = SessionDB::open(&path).unwrap();
    let now = NOW + 301;
    let expected = db.assistant_im_poll_lease(now).unwrap().unwrap();
    for (id, timestamp) in [(110, Some(NOW)), (111, None), (112, Some(now + 31))] {
        let mut message = input(id, TelegramInputKind::Message("do old work".into()));
        message.sent_at = timestamp;
        assert_eq!(
            db.receive_assistant_im_update(&expected, message, now)
                .unwrap(),
            IngressOutcome::Ignored
        );
    }
    assert!(db.assistant_im_pending_commands().unwrap().is_empty());
    assert_eq!(
        db.assistant_im_poll_lease(now).unwrap().unwrap().cursor,
        113
    );
    let mut message = input(113, TelegramInputKind::Message("new request".into()));
    message.sent_at = Some(now);
    assert_eq!(
        db.receive_assistant_im_update(&expected, message, now)
            .unwrap(),
        IngressOutcome::Queued
    );
    assert_eq!(db.assistant_im_pending_commands().unwrap().len(), 1);
}

#[test]
fn only_unpublished_bot_claims_may_be_rolled_back() {
    let (_directory, _path, db) = database();
    db.reserve_telegram_bot(201, "pending").unwrap();
    assert!(db
        .claim_telegram_bot(201, BotPurpose::PersonalAssistant, "assistant")
        .is_err());
    db.discard_telegram_bot_reservation(201, "different-account")
        .unwrap();
    assert!(db
        .claim_telegram_bot(201, BotPurpose::PersonalAssistant, "assistant")
        .is_err());
    db.discard_telegram_bot_reservation(201, "pending").unwrap();
    db.claim_telegram_bot(201, BotPurpose::PersonalAssistant, "assistant")
        .unwrap();
    db.reserve_telegram_bot(202, "published").unwrap();
    db.claim_telegram_bot(202, BotPurpose::Ordinary, "published")
        .unwrap();
    db.discard_telegram_bot_reservation(202, "published")
        .unwrap();
    assert!(db
        .claim_telegram_bot(202, BotPurpose::PersonalAssistant, "assistant")
        .is_err());
}

fn confirm_candidate(db: &SessionDB) -> PollLease {
    let code = db.begin_assistant_im_pairing(NOW).unwrap();
    let expected = db.assistant_im_poll_lease(NOW).unwrap().unwrap();
    assert_eq!(
        db.receive_assistant_im_update(
            &expected,
            input(10, TelegramInputKind::PairCode(code.code)),
            NOW
        )
        .unwrap(),
        IngressOutcome::PairingCandidate
    );
    db.confirm_assistant_im_pairing(expected.generation, OWNER, OWNER, NOW)
        .unwrap();
    expected
}

#[test]
fn secondary_confirmation_is_claimed_once_by_another_process_and_not_replayed_after_crash() {
    let (_directory, path, secondary) = database();
    connected(&secondary);
    let lease = confirm_candidate(&secondary);
    let primary = SessionDB::open(&path).unwrap();
    let activation = primary
        .prepare_assistant_im_activation(NOW)
        .unwrap()
        .unwrap();
    assert_eq!(activation.prompt.lease.connection_id, lease.connection_id);
    assert_eq!(activation.prompt.lease.generation, lease.generation);
    assert_eq!(activation.prompt.chat_id, OWNER);
    assert!(secondary
        .prepare_assistant_im_activation(NOW)
        .unwrap()
        .is_none());
    primary
        .with_conn_for_test(|conn| {
            let persisted: String = conn.query_row(
                "SELECT callback_data FROM assistant_im_activations WHERE id = ?1",
                [&activation.id],
                |row| row.get(0),
            )?;
            assert!(persisted.is_empty());
            Ok(())
        })
        .unwrap();
    drop(primary);
    let primary = SessionDB::open(&path).unwrap();
    assert_eq!(primary.recover_assistant_im_activations().unwrap(), 1);
    assert!(primary
        .prepare_assistant_im_activation(NOW)
        .unwrap()
        .is_none());
    assert_eq!(
        primary.assistant_im_delivery_receipts().unwrap()[0].state,
        DeliveryState::Unknown
    );
    // An unknown HTTP outcome may actually have delivered. A valid one-use
    // callback can still activate, but no worker may send the button again.
    let nonce = activation
        .prompt
        .callback_data
        .strip_prefix(ACTIVATE_PREFIX)
        .unwrap()
        .to_owned();
    assert_eq!(
        primary
            .receive_assistant_im_update(&lease, input(11, TelegramInputKind::Activate(nonce)), NOW)
            .unwrap(),
        IngressOutcome::Activated
    );
}

#[test]
fn activation_pending_is_cancelled_on_revoke_expiry_and_repair() {
    for action in ["revoke", "expiry", "repair"] {
        let (_directory, _path, db) = database();
        connected(&db);
        let old = confirm_candidate(&db);
        let now = match action {
            "revoke" => {
                db.revoke_assistant_im_pairing().unwrap();
                NOW
            }
            "expiry" => NOW + 600,
            _ => {
                db.begin_assistant_im_pairing(NOW).unwrap();
                NOW
            }
        };
        assert!(db.prepare_assistant_im_activation(now).unwrap().is_none());
        db.with_conn_for_test(|conn| {
            let (state, callback): (String, String) = conn.query_row(
                "SELECT state,callback_data FROM assistant_im_activations WHERE generation = ?1",
                [old.generation],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            assert_eq!(state, "cancelled");
            assert!(callback.is_empty());
            Ok(())
        })
        .unwrap();
        assert!(db
            .confirm_assistant_im_pairing(old.generation, OWNER, OWNER, now)
            .is_err());
    }
}

#[test]
fn activation_accepted_receipt_never_requeues_the_same_confirmation() {
    let (_directory, _path, db) = database();
    connected(&db);
    let lease = confirm_candidate(&db);
    let activation = db.prepare_assistant_im_activation(NOW).unwrap().unwrap();
    db.finish_assistant_im_activation(&activation.id, Some(9))
        .unwrap();
    assert_eq!(db.recover_assistant_im_activations().unwrap(), 0);
    assert!(db.prepare_assistant_im_activation(NOW).unwrap().is_none());
    assert!(db
        .confirm_assistant_im_pairing(lease.generation, OWNER, OWNER, NOW)
        .is_err());
    let receipt = &db.assistant_im_delivery_receipts().unwrap()[0];
    assert_eq!(receipt.state, DeliveryState::Accepted);
    assert_eq!(receipt.message_id, Some(9));
}

fn timestamp(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .unwrap()
        .timestamp()
}

fn notifications(db: &SessionDB, now: i64) -> (PollLease, SessionGrant) {
    connected(db);
    let expected = activate(db, 10);
    db.set_assistant_im_notifications_paused(false).unwrap();
    let permission = db
        .set_assistant_im_grant(
            SessionGrant {
                session_id: "work".into(),
                revision: 0,
                query_status: true,
                delegate: false,
                stop: false,
                completion_notice: true,
                expires_at: now + 7 * 86_400,
            },
            now,
        )
        .unwrap();
    (expected, permission)
}

fn enqueue_notice(
    db: &SessionDB,
    lease: &PollLease,
    permission: &SessionGrant,
    id: &str,
    now: i64,
) -> String {
    db.enqueue_assistant_im_reply(
        lease,
        id,
        "work",
        permission.revision,
        true,
        "Work completed. View details in Hope.",
        now,
    )
    .unwrap()
}

#[test]
fn notification_quiet_hours_include_22_and_end_exactly_at_08() {
    let policy = NotificationPolicy::from_timezone(Some("Asia/Shanghai"));
    for (at, allowed) in [
        ("2026-11-09T21:59:59+08:00", true),
        ("2026-11-09T22:00:00+08:00", false),
        ("2026-11-10T07:59:59+08:00", false),
        ("2026-11-10T08:00:00+08:00", true),
    ] {
        let (_directory, _path, db) = database();
        let now = timestamp(at);
        let (lease, permission) = notifications(&db, now);
        let id = enqueue_notice(&db, &lease, &permission, "completion", now);
        assert_eq!(
            db.prepare_assistant_im_delivery_with_policy(now, &policy)
                .unwrap()
                .is_some(),
            allowed,
            "{at}"
        );
        let rows = db
            .assistant_im_delivery_receipts_with_policy(now, &policy)
            .unwrap();
        let receipt = rows.iter().find(|receipt| receipt.id == id).unwrap();
        assert_eq!(
            receipt.deferred_reason,
            (!allowed).then_some(DeliveryDeferredReason::QuietHours)
        );
        if !allowed {
            let reply = db
                .enqueue_assistant_im_reply(
                    &lease,
                    "direct",
                    "work",
                    permission.revision,
                    false,
                    "Current status",
                    now,
                )
                .unwrap();
            assert_eq!(
                db.prepare_assistant_im_delivery_with_policy(now, &policy)
                    .unwrap()
                    .unwrap()
                    .id,
                reply
            );
        }
    }
}

#[test]
fn notification_missing_timezone_never_blocks_immediate_reply_and_expiry_still_cancels() {
    for timezone in [None, Some("not/a-timezone")] {
        let policy = NotificationPolicy::from_timezone(timezone);
        assert!(policy.timezone.is_none());
        let (_directory, _path, db) = database();
        let now = timestamp("2026-11-09T12:00:00+08:00");
        let (lease, permission) = notifications(&db, now);
        let notice = enqueue_notice(&db, &lease, &permission, "completion", now);
        let reply = db
            .enqueue_assistant_im_reply(
                &lease,
                "direct-reply",
                "work",
                permission.revision,
                false,
                "Current status",
                now + 1,
            )
            .unwrap();
        assert_eq!(
            db.prepare_assistant_im_delivery_with_policy(now + 1, &policy)
                .unwrap()
                .unwrap()
                .id,
            reply
        );
        assert!(db
            .prepare_assistant_im_delivery_with_policy(now + 1, &policy)
            .unwrap()
            .is_none());
        let rows = db
            .assistant_im_delivery_receipts_with_policy(now + 1, &policy)
            .unwrap();
        assert_eq!(
            rows.iter()
                .find(|row| row.id == notice)
                .unwrap()
                .deferred_reason,
            Some(DeliveryDeferredReason::TimezoneRequired)
        );
        assert!(rows
            .iter()
            .find(|row| row.id == reply)
            .unwrap()
            .deferred_reason
            .is_none());
        assert!(db
            .prepare_assistant_im_delivery_with_policy(now + 86_400, &policy)
            .unwrap()
            .is_none());
        let rows = db
            .assistant_im_delivery_receipts_with_policy(now + 86_400, &policy)
            .unwrap();
        assert_eq!(
            rows.iter().find(|row| row.id == notice).unwrap().state,
            DeliveryState::Cancelled
        );
        db.with_conn_for_test(|conn| {
            assert_eq!(
                conn.query_row(
                    "SELECT COUNT(*) FROM assistant_im_notification_attempts",
                    [],
                    |row| row.get::<_, i64>(0)
                )?,
                0
            );
            Ok(())
        })
        .unwrap();
    }
}

#[test]
fn notification_concurrent_claims_and_unknown_sends_share_two_daily_slots() {
    let (_directory, path, db) = database();
    let now = timestamp("2026-11-09T12:00:00+08:00");
    let policy = NotificationPolicy::from_timezone(Some("Asia/Shanghai"));
    let (lease, permission) = notifications(&db, now);
    enqueue_notice(&db, &lease, &permission, "first", now);
    let first = db
        .prepare_assistant_im_delivery_with_policy(now, &policy)
        .unwrap()
        .unwrap();
    db.finish_assistant_im_delivery(&first.id, None).unwrap();
    enqueue_notice(&db, &lease, &permission, "second", now);
    enqueue_notice(&db, &lease, &permission, "third", now);
    let other = SessionDB::open(&path).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let first_barrier = barrier.clone();
    let first_policy = policy.clone();
    let left = std::thread::spawn(move || {
        first_barrier.wait();
        db.prepare_assistant_im_delivery_with_policy(now, &first_policy)
    });
    let right = std::thread::spawn(move || {
        barrier.wait();
        other.prepare_assistant_im_delivery_with_policy(now, &policy)
    });
    let prepared = [
        left.join().unwrap().unwrap(),
        right.join().unwrap().unwrap(),
    ];
    assert_eq!(prepared.iter().filter(|result| result.is_some()).count(), 1);
    let db = SessionDB::open(&path).unwrap();
    assert_eq!(db.recover_assistant_im_deliveries().unwrap(), 1);
    let policy = NotificationPolicy::from_timezone(Some("Asia/Shanghai"));
    assert!(db
        .prepare_assistant_im_delivery_with_policy(now, &policy)
        .unwrap()
        .is_none());
    let rows = db
        .assistant_im_delivery_receipts_with_policy(now, &policy)
        .unwrap();
    assert_eq!(
        rows.iter()
            .filter(|row| row.state == DeliveryState::Unknown)
            .count(),
        2
    );
    assert!(rows
        .iter()
        .any(|row| row.deferred_reason == Some(DeliveryDeferredReason::DailyLimit)));
    // A foreground answer remains deliverable after the daily slots are spent.
    let reply = db
        .enqueue_assistant_im_reply(
            &lease,
            "direct",
            "work",
            permission.revision,
            false,
            "Current status",
            now,
        )
        .unwrap();
    assert_eq!(
        db.prepare_assistant_im_delivery_with_policy(now, &policy)
            .unwrap()
            .unwrap()
            .id,
        reply
    );
    db.revoke_assistant_im_pairing().unwrap();
    db.with_conn_for_test(|conn| {
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM assistant_im_notification_attempts",
                [],
                |row| row.get::<_, i64>(0)
            )?,
            2
        );
        // Even source deletion/cascading delivery removal cannot refund attempts.
        conn.execute("DELETE FROM sessions WHERE id = 'work'", [])?;
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM assistant_im_notification_attempts",
                [],
                |row| row.get::<_, i64>(0)
            )?,
            2
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn notification_daily_budget_uses_local_dates_across_midnight_and_dst() {
    let policy = NotificationPolicy::from_timezone(Some("America/New_York"));
    for (previous, current) in [
        ("2026-03-07T21:00:00-05:00", "2026-03-08T08:00:00-04:00"),
        ("2026-10-31T21:00:00-04:00", "2026-11-01T08:00:00-05:00"),
    ] {
        let (_directory, _path, db) = database();
        let before = timestamp(previous);
        let now = timestamp(current);
        let (lease, permission) = notifications(&db, before);
        for id in ["previous-one", "previous-two"] {
            enqueue_notice(&db, &lease, &permission, id, before);
            let send = db
                .prepare_assistant_im_delivery_with_policy(before, &policy)
                .unwrap()
                .unwrap();
            db.finish_assistant_im_delivery(&send.id, Some(1)).unwrap();
        }
        enqueue_notice(&db, &lease, &permission, "next-day", before);
        assert!(db
            .prepare_assistant_im_delivery_with_policy(before, &policy)
            .unwrap()
            .is_none());
        assert!(
            db.prepare_assistant_im_delivery_with_policy(now, &policy)
                .unwrap()
                .is_some(),
            "{current}"
        );
    }
}

#[test]
fn ordinary_bot_reservations_recover_crashes_before_and_after_config_publication() {
    let (_directory, path, db) = database();
    db.reserve_telegram_bot(301, "never-published").unwrap();
    db.reserve_telegram_bot(302, "published").unwrap();
    db.claim_telegram_bot(303, BotPurpose::Ordinary, "used-before")
        .unwrap();
    drop(db);
    let restarted = SessionDB::open(&path).unwrap();
    let observer = SessionDB::open(&path).unwrap();
    let pending = restarted
        .reconcile_telegram_bot_reservations(&["published".into()])
        .unwrap();
    assert_eq!(pending, vec![(302, "published".to_owned())]);
    // A crash before publication cannot permanently occupy an invisible id.
    observer
        .claim_telegram_bot(301, BotPurpose::Ordinary, "retry")
        .unwrap();
    // A published but uncommitted account remains protected until getMe proves
    // its current identity; account-id presence alone cannot commit the claim.
    assert!(observer
        .claim_telegram_bot(302, BotPurpose::PersonalAssistant, "private")
        .is_err());
    restarted
        .with_conn_for_test(|conn| {
            assert_eq!(
                conn.query_row(
                    "SELECT committed FROM im_bot_purposes WHERE subject_id=302",
                    [],
                    |row| row.get::<_, i64>(0)
                )?,
                0
            );
            Ok(())
        })
        .unwrap();
    restarted
        .commit_ordinary_telegram_bot(302, "published")
        .unwrap();
    assert!(observer
        .reconcile_telegram_bot_reservations(&[])
        .unwrap()
        .is_empty());
    assert!(observer
        .claim_telegram_bot(302, BotPurpose::PersonalAssistant, "private")
        .is_err());
    assert!(observer
        .claim_telegram_bot(303, BotPurpose::PersonalAssistant, "private")
        .is_err());
}

#[test]
fn ordinary_token_replacement_releases_old_binding_without_erasing_used_purpose() {
    let (_directory, path, db) = database();
    db.claim_telegram_bot(401, BotPurpose::Ordinary, "account")
        .unwrap();
    // This token was staged, but config still points to bot 401 after a crash.
    db.reserve_telegram_bot(402, "account").unwrap();
    let other = SessionDB::open(&path).unwrap();
    assert_eq!(
        other
            .reconcile_telegram_bot_reservations(&["account".into()])
            .unwrap(),
        vec![(402, "account".to_owned())]
    );
    other.commit_ordinary_telegram_bot(401, "account").unwrap();
    // Canonical getMe for the actual current config proves bot 402 was never
    // published; its provisional purpose can safely be discarded.
    db.claim_telegram_bot(402, BotPurpose::PersonalAssistant, "private")
        .unwrap();
    db.reserve_telegram_bot(403, "account").unwrap();
    db.commit_ordinary_telegram_bot(403, "account").unwrap();
    other
        .claim_telegram_bot(401, BotPurpose::Ordinary, "new-ordinary-account")
        .unwrap();
    other
        .release_telegram_bot(401, "new-ordinary-account")
        .unwrap();
    assert!(db
        .claim_telegram_bot(401, BotPurpose::PersonalAssistant, "private")
        .is_err());
    assert!(db
        .claim_telegram_bot(403, BotPurpose::Ordinary, "different-account")
        .is_err());
}
