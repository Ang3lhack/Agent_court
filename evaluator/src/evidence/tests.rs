use super::*;
use crate::{Check, CRITERIA, TEST_SOURCE};
use std::path::PathBuf;

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        Self(crate::temporary_workspace().unwrap())
    }

    fn package(&self, record: &EvidenceRecord) -> PathBuf {
        self.0.join("attempts").join(&record.attempt_id)
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        // Only test-owned temporary directories, never the repository's evidence.
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn sample() -> Evaluation {
    Evaluation::from_checks(
        Status::Pass,
        vec![Check {
            name: "counter_contract".into(),
            status: Status::Pass,
            detail: "Test-only report for persistence checks".into(),
        }],
    )
}

fn package_bytes(directory: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path()).unwrap(),
            )
        })
        .collect();
    files.sort();
    files
}

fn legacy_record() -> EvidenceRecord {
    let report = sample().report;
    EvidenceRecord {
        mission_id: report.mission_id,
        attempt_id: "legacy-1".into(),
        previous_attempt_id: None,
        criteria_hash: report.criteria_hash,
        artifact_hash: report.artifact_hash,
        report_hash: report.report_hash,
        verdict: report.verdict,
        timestamp_unix_ms: report.timestamp_unix_ms,
        evidence_format_version: None,
    }
}

#[test]
fn complete_real_evaluation_archives_exact_bytes_and_manifest_hashes() {
    let root = TestRoot::new();
    let evaluation = crate::evaluate();
    assert_eq!(evaluation.report.verdict, Status::Pass);
    let record = record_attempt(&root.0, &evaluation).unwrap();
    let directory = root.package(&record);
    assert_eq!(package_bytes(&directory).len(), 5);
    let manifest: Manifest =
        serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest.record, record);
    assert_eq!(
        record.evidence_format_version,
        Some(EVIDENCE_FORMAT_VERSION)
    );
    assert_eq!(
        fs::read(directory.join("criteria.txt")).unwrap(),
        CRITERIA.as_bytes()
    );
    assert_eq!(
        fs::read(directory.join("artifact.rs")).unwrap(),
        TEST_SOURCE.as_bytes()
    );
    for (name, digest) in [
        ("criteria.txt", &manifest.record.criteria_hash),
        ("artifact.rs", &manifest.record.artifact_hash),
        ("report-payload.json", &manifest.record.report_hash),
        ("report.json", &manifest.report_file_hash),
    ] {
        assert_eq!(sha256(&fs::read(directory.join(name)).unwrap()), *digest);
    }
    let payload: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("report-payload.json")).unwrap()).unwrap();
    assert!(payload.get("report_hash").is_none());
    assert_eq!(payload, evaluation.report.hash_payload());
    assert_eq!(
        record.timestamp_unix_ms,
        evaluation.report.timestamp_unix_ms
    );
    assert!(matches!(
        verify_attempt(&root.0, &record.attempt_id),
        Ok(Verification::Verified(_))
    ));
}

#[test]
fn counter_criteria_and_fixture_hashes_remain_compatible_with_history() {
    let evaluation = sample();
    assert_eq!(
        evaluation.report.criteria_hash,
        "7436340ba6690040ea50fdcf786ed1c23b9595f326d895002f560538eb23d28f"
    );
    assert_eq!(
        evaluation.report.artifact_hash,
        "5ad72d8d4c8bae96ccd0979229a3d98f6d3d582fe85625da83a706a51efc9d57"
    );
}

#[test]
fn modified_artifact_is_detected() {
    let root = TestRoot::new();
    let record = record_attempt(&root.0, &sample()).unwrap();
    fs::write(
        root.package(&record).join("artifact.rs"),
        b"modified artifact",
    )
    .unwrap();
    assert!(verify_attempt(&root.0, &record.attempt_id)
        .err()
        .unwrap()
        .contains("SHA-256 mismatch for artifact.rs"));
}

#[test]
fn payload_whitespace_changes_are_detected_without_reserializing_json() {
    let root = TestRoot::new();
    let record = record_attempt(&root.0, &sample()).unwrap();
    let path = root.package(&record).join("report-payload.json");
    OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(b"\n")
        .unwrap();
    assert!(verify_attempt(&root.0, &record.attempt_id)
        .err()
        .unwrap()
        .contains("SHA-256 mismatch for report-payload.json"));
}

#[test]
fn modified_criteria_are_detected() {
    let root = TestRoot::new();
    let record = record_attempt(&root.0, &sample()).unwrap();
    fs::write(
        root.package(&record).join("criteria.txt"),
        b"different criteria",
    )
    .unwrap();
    assert!(verify_attempt(&root.0, &record.attempt_id)
        .err()
        .unwrap()
        .contains("SHA-256 mismatch for criteria.txt"));
}

#[test]
fn full_report_whitespace_changes_are_detected() {
    let root = TestRoot::new();
    let record = record_attempt(&root.0, &sample()).unwrap();
    OpenOptions::new()
        .append(true)
        .open(root.package(&record).join("report.json"))
        .unwrap()
        .write_all(b"\n")
        .unwrap();
    assert!(verify_attempt(&root.0, &record.attempt_id)
        .err()
        .unwrap()
        .contains("SHA-256 mismatch for report.json"));
}

#[test]
fn missing_fundamental_files_are_detected() {
    for name in [
        "criteria.txt",
        "artifact.rs",
        "report-payload.json",
        "report.json",
        "manifest.json",
    ] {
        let root = TestRoot::new();
        let record = record_attempt(&root.0, &sample()).unwrap();
        fs::remove_file(root.package(&record).join(name)).unwrap();
        assert!(verify_attempt(&root.0, &record.attempt_id)
            .err()
            .unwrap()
            .contains(name));
    }
}

#[test]
fn previous_attempts_and_log_bytes_are_preserved() {
    let root = TestRoot::new();
    let first = record_attempt(&root.0, &sample()).unwrap();
    let log = fs::read(root.0.join(LOG_NAME)).unwrap();
    let first_files = package_bytes(&root.package(&first));
    let second = record_attempt(&root.0, &sample()).unwrap();
    assert_ne!(first.attempt_id, second.attempt_id);
    assert_eq!(second.previous_attempt_id.as_ref(), Some(&first.attempt_id));
    assert!(fs::read(root.0.join(LOG_NAME)).unwrap().starts_with(&log));
    assert_eq!(package_bytes(&root.package(&first)), first_files);
    assert!(matches!(
        verify_attempt(&root.0, &first.attempt_id),
        Ok(Verification::Verified(_))
    ));
    assert!(matches!(
        verify_attempt(&root.0, &second.attempt_id),
        Ok(Verification::Verified(_))
    ));
}

#[test]
fn historical_metadata_remains_unverifiable_and_its_exact_bytes_are_preserved() {
    let root = TestRoot::new();
    let historical = legacy_record();
    let bytes = format!("  {} \r\n", serde_json::to_string(&historical).unwrap()).into_bytes();
    fs::write(root.0.join(LOG_NAME), &bytes).unwrap();
    assert!(matches!(
        verify_attempt(&root.0, &historical.attempt_id),
        Ok(Verification::Legacy(_))
    ));
    assert!(!root.0.join("attempts").exists());
    let current = record_attempt(&root.0, &sample()).unwrap();
    assert_eq!(
        current.previous_attempt_id,
        Some(historical.attempt_id.clone())
    );
    assert!(fs::read(root.0.join(LOG_NAME)).unwrap().starts_with(&bytes));
    assert!(!root.package(&historical).exists());
    assert!(matches!(
        verify_attempt(&root.0, &current.attempt_id),
        Ok(Verification::Verified(_))
    ));
    assert!(matches!(
        verify_attempt(&root.0, &historical.attempt_id),
        Ok(Verification::Legacy(_))
    ));
}

#[test]
fn lost_new_packages_do_not_become_legacy_successes() {
    let root = TestRoot::new();
    let record = record_attempt(&root.0, &sample()).unwrap();
    fs::remove_dir_all(root.package(&record)).unwrap();
    assert!(verify_attempt(&root.0, &record.attempt_id).is_err());
}

#[test]
fn uncommitted_packages_are_not_complete_attempts() {
    let root = TestRoot::new();
    fs::create_dir_all(root.0.join("attempts/incomplete")).unwrap();
    fs::write(root.0.join("attempts/incomplete/manifest.json"), b"{}").unwrap();
    assert!(verify_attempt(&root.0, "incomplete")
        .err()
        .unwrap()
        .contains("not committed"));
    assert!(!root.0.join(LOG_NAME).exists());
}

#[test]
fn verification_never_writes_evidence_or_creates_a_missing_store() {
    let root = TestRoot::new();
    let absent = root.0.join("absent");
    assert!(verify_attempt(&absent, "missing").is_err());
    assert!(!absent.exists());
    let record = record_attempt(&root.0, &sample()).unwrap();
    let files = package_bytes(&root.package(&record));
    let log = fs::read(root.0.join(LOG_NAME)).unwrap();
    verify_attempt(&root.0, &record.attempt_id).unwrap();
    assert_eq!(files, package_bytes(&root.package(&record)));
    assert_eq!(log, fs::read(root.0.join(LOG_NAME)).unwrap());
}

#[test]
fn invalid_ids_cannot_escape_the_evidence_directory() {
    let root = TestRoot::new();
    for id in [
        "../outside",
        "/tmp/outside",
        "",
        ".",
        "with/slash",
        &"x".repeat(129),
    ] {
        assert!(verify_attempt(&root.0, id)
            .err()
            .unwrap()
            .contains("invalid attempt ID"));
    }
}

#[test]
fn directory_creation_failures_do_not_publish_records() {
    let root = TestRoot::new();
    let blocked = root.0.join("blocked");
    fs::write(&blocked, b"existing file").unwrap();
    assert!(record_attempt(&blocked, &sample()).is_err());
    assert_eq!(fs::read(blocked).unwrap(), b"existing file");
    assert!(!root.0.join(LOG_NAME).exists());
}

#[test]
fn package_persistence_failure_leaves_history_untouched() {
    let root = TestRoot::new();
    let bytes = serde_json::to_vec(&legacy_record()).unwrap();
    fs::write(root.0.join(LOG_NAME), &bytes).unwrap();
    fs::write(root.0.join("attempts"), b"blocks directory creation").unwrap();
    assert!(record_attempt(&root.0, &sample()).is_err());
    assert_eq!(fs::read(root.0.join(LOG_NAME)).unwrap(), bytes);
}

#[test]
fn index_persistence_failure_preserves_the_old_committed_index() {
    let root = TestRoot::new();
    let first = record_attempt(&root.0, &sample()).unwrap();
    let bytes = fs::read(root.0.join(LOG_NAME)).unwrap();
    let mut rejected = first.clone();
    rejected.attempt_id = "rejected".into();
    let pending = root.0.join(".evidence-rejected.pending");
    fs::write(&pending, b"already exists").unwrap();
    assert!(commit_record(&root.0, &bytes, &rejected).is_err());
    assert_eq!(fs::read(root.0.join(LOG_NAME)).unwrap(), bytes);
    assert_eq!(fs::read(pending).unwrap(), b"already exists");
    assert!(matches!(
        verify_attempt(&root.0, &first.attempt_id),
        Ok(Verification::Verified(_))
    ));
    assert!(verify_attempt(&root.0, &rejected.attempt_id).is_err());
}

#[test]
fn malformed_history_is_reported_and_never_repaired_automatically() {
    let root = TestRoot::new();
    let bytes = b"{\"mission_id\":\"truncated";
    fs::write(root.0.join(LOG_NAME), bytes).unwrap();
    assert!(record_attempt(&root.0, &sample())
        .err()
        .unwrap()
        .contains("invalid evidence log"));
    assert_eq!(fs::read(root.0.join(LOG_NAME)).unwrap(), bytes);
    assert!(!root.0.join("attempts").exists());
}

#[test]
fn duplicate_history_ids_are_rejected() {
    let root = TestRoot::new();
    let line = serde_json::to_string(&legacy_record()).unwrap();
    fs::write(root.0.join(LOG_NAME), format!("{line}\n{line}\n")).unwrap();
    assert!(record_attempt(&root.0, &sample())
        .err()
        .unwrap()
        .contains("duplicate attempt ID"));
}

#[test]
fn concurrent_writers_are_rejected_and_the_os_lock_is_released() {
    let root = TestRoot::new();
    let lock = writer_lock(&root.0).unwrap();
    assert!(record_attempt(&root.0, &sample())
        .err()
        .unwrap()
        .contains("writer lock"));
    assert!(!root.0.join(LOG_NAME).exists());
    drop(lock);
    assert!(record_attempt(&root.0, &sample()).is_ok());
}

#[test]
fn writer_unlocks_even_if_another_handle_still_references_the_file_description() {
    let root = TestRoot::new();
    let lock = writer_lock(&root.0).unwrap();
    // Model a file descriptor inherited by a child before close-on-exec.
    let inherited = lock.0.try_clone().unwrap();
    drop(lock);
    assert!(record_attempt(&root.0, &sample()).is_ok());
    drop(inherited);
}

#[test]
fn existing_files_are_never_overwritten() {
    let root = TestRoot::new();
    let file = root.0.join("artifact.rs");
    fs::write(&file, b"original").unwrap();
    assert!(write_new_file(&file, b"replacement", None).is_err());
    assert_eq!(fs::read(file).unwrap(), b"original");
}

#[test]
fn declared_hashes_are_checked_before_any_package_is_recorded() {
    let root = TestRoot::new();
    let mut evaluation = sample();
    evaluation.artifact.push(b' ');
    assert!(record_attempt(&root.0, &evaluation)
        .err()
        .unwrap()
        .contains("artifact.rs"));
    assert!(!root.0.join(LOG_NAME).exists());
    assert!(!root.0.join("attempts").exists());
}

#[test]
fn manifest_metadata_changes_are_detected_against_the_index() {
    let root = TestRoot::new();
    let record = record_attempt(&root.0, &sample()).unwrap();
    let path = root.package(&record).join("manifest.json");
    let mut manifest: Manifest = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest.record.verdict = Status::Fail;
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(verify_attempt(&root.0, &record.attempt_id)
        .err()
        .unwrap()
        .contains("does not match evidence.jsonl"));
}

#[test]
fn report_semantics_must_match_even_if_its_extra_checksum_is_recomputed() {
    let root = TestRoot::new();
    let record = record_attempt(&root.0, &sample()).unwrap();
    let directory = root.package(&record);
    let mut report: EvaluationReport =
        serde_json::from_slice(&fs::read(directory.join("report.json")).unwrap()).unwrap();
    report.isolation = "different isolation claim".into();
    let bytes = serde_json::to_vec(&report).unwrap();
    fs::write(directory.join("report.json"), &bytes).unwrap();
    let manifest_path = directory.join("manifest.json");
    let mut manifest: Manifest =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest.report_file_hash = sha256(&bytes);
    fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(verify_attempt(&root.0, &record.attempt_id)
        .err()
        .unwrap()
        .contains("hash payload"));
}
