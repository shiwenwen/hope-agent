//! Fast contracts shared by the macOS, Linux, and Windows CI lanes.
//!
//! Keep these tests in one integration-test binary: they exercise the public
//! cross-platform boundary independently of module unit tests without paying
//! the linker cost of one binary per primitive.

use std::fs;
use std::io::ErrorKind;

use ha_core::platform;
use ha_core::session::SessionDB;
use ha_core::workflow_mode::WorkflowMode;

#[test]
fn atomic_write_and_create_new_preserve_the_publish_contract() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("nested").join("document.txt");

    platform::write_atomic(&path, b"first").expect("initial atomic write");
    platform::write_atomic(&path, b"second").expect("replacement atomic write");
    assert_eq!(fs::read(&path).expect("read replacement"), b"second");

    let create_only = dir.path().join("create-only.txt");
    platform::write_atomic_create_new(&create_only, b"winner").expect("create new file");
    let error = platform::write_atomic_create_new(&create_only, b"loser")
        .expect_err("second create must not clobber the winner");
    assert_eq!(error.kind(), ErrorKind::AlreadyExists);
    assert_eq!(fs::read(create_only).expect("read winner"), b"winner");
}

#[test]
fn staged_publication_honors_the_overwrite_flag() {
    let dir = tempfile::tempdir().expect("tempdir");
    let source = dir.path().join("staged.bin");
    let target = dir.path().join("published.bin");
    fs::write(&source, b"new").expect("write staged file");
    fs::write(&target, b"old").expect("write existing target");

    let error = platform::publish_atomic_file(&source, &target, false)
        .expect_err("no-overwrite publish must preserve the target");
    assert_eq!(error.kind(), ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&source).expect("staged file remains"), b"new");
    assert_eq!(fs::read(&target).expect("old target remains"), b"old");

    platform::publish_atomic_file(&source, &target, true).expect("overwrite publication");
    assert!(!source.exists());
    assert_eq!(fs::read(target).expect("read published file"), b"new");
}

#[test]
fn exclusive_lock_can_be_reacquired_after_drop() {
    let dir = tempfile::tempdir().expect("tempdir");
    let lock_path = dir.path().join("locks").join("primary.lock");

    let first = platform::try_acquire_exclusive_lock(&lock_path)
        .expect("first lock attempt")
        .expect("first caller owns lock");
    assert!(
        platform::try_acquire_exclusive_lock(&lock_path)
            .expect("contended lock attempt")
            .is_none(),
        "a second writer must observe contention"
    );

    drop(first);
    assert!(
        platform::try_acquire_exclusive_lock(&lock_path)
            .expect("lock attempt after drop")
            .is_some(),
        "dropping the handle must release the OS lock"
    );
}

#[test]
fn atomic_binary_replacement_publishes_the_complete_source() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("hope-agent.bin");
    let source = dir.path().join("hope-agent.new");
    fs::write(&target, b"old binary").expect("write old binary");
    fs::write(&source, b"new binary").expect("write new binary");

    platform::atomic_replace_binary(&target, &source).expect("replace binary");

    assert_eq!(fs::read(target).expect("read replacement"), b"new binary");
    assert!(!source.exists());
}

#[test]
fn native_cross_device_errors_are_recognized() {
    #[cfg(unix)]
    let raw_error = 18; // EXDEV
    #[cfg(windows)]
    let raw_error = 17; // ERROR_NOT_SAME_DEVICE

    assert!(platform::is_cross_device_rename(
        &std::io::Error::from_raw_os_error(raw_error)
    ));
    assert!(!platform::is_cross_device_rename(
        &std::io::Error::from_raw_os_error(2)
    ));
}

#[test]
fn durable_session_database_survives_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("sessions.db");
    let session_id = {
        let db = SessionDB::open(&db_path).expect("open durable session database");
        db.create_session(ha_core::agent_loader::DEFAULT_AGENT_ID)
            .expect("create session")
            .id
    };

    let reopened = SessionDB::open(&db_path).expect("reopen durable session database");
    assert_eq!(
        reopened
            .get_session_workflow_mode(&session_id)
            .expect("read persisted session"),
        Some(WorkflowMode::Off),
        "a committed production session must survive reopening",
    );
}
