use super::*;
use crate::{sha256, Evaluation};
use std::{fs, path::PathBuf};

struct Package {
    root: PathBuf,
    record: EvidenceRecord,
}

impl Package {
    fn new() -> Self {
        let root = crate::temporary_workspace().unwrap();
        let record =
            evidence::record_attempt(&root, &Evaluation::from_checks(Status::Pass, vec![]))
                .unwrap();
        Self { root, record }
    }
    fn input(&self) -> AttemptInput {
        AttemptInput::from_record(self.record.clone()).unwrap()
    }
    fn publish(&self, mock: &mut Mock) -> Result<Outcome, PublishError> {
        publish(&self.root, &self.record.attempt_id, mock)
    }
}

impl Drop for Package {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn remote(input: &AttemptInput) -> RemoteAttempt {
    RemoteAttempt {
        mission_id: input.mission_id.clone(),
        attempt_id: input.attempt_id.clone(),
        previous_attempt_id: input.previous_attempt_id.clone(),
        criteria_hash: input.criteria_hash.clone(),
        artifact_hash: input.artifact_hash.clone(),
        report_hash: input.report_hash.clone(),
        verdict: input.verdict,
        timestamp_unix_ms: input.timestamp_unix_ms,
        registered_at: 42,
        registered_by: Principal::anonymous(),
    }
}

struct Mock {
    mission: Option<Mission>,
    initial: Option<RemoteAttempt>,
    after: Option<RemoteAttempt>,
    history: Vec<RemoteAttempt>,
    error: Option<PublishError>,
    write_response: Option<RemoteAttempt>,
    store_on_write: bool,
    fail_confirmation: bool,
    calls: Vec<&'static str>,
    writes: Vec<AttemptInput>,
    pages: usize,
}

impl Mock {
    fn new(package: &Package) -> Self {
        Self {
            mission: Some(Mission {
                mission_id: package.record.mission_id.clone(),
                name: "Counter".into(),
                description: String::new(),
                criteria_hash: package.record.criteria_hash.clone(),
                created_at: 1,
                registered_by: Principal::anonymous(),
            }),
            initial: None,
            after: None,
            history: vec![],
            error: None,
            write_response: None,
            store_on_write: true,
            fail_confirmation: false,
            calls: vec![],
            writes: vec![],
            pages: 0,
        }
    }
}

impl Registry for Mock {
    fn get_mission(&mut self, _: &str) -> Result<Option<Mission>, PublishError> {
        self.calls.push("get_mission");
        Ok(self.mission.clone())
    }
    fn get_attempt(&mut self, _: &str) -> Result<Option<RemoteAttempt>, PublishError> {
        self.calls.push("get_attempt");
        if self.writes.is_empty() {
            return Ok(self.initial.clone());
        }
        if self.fail_confirmation {
            return Err(PublishError::Cli(CliFailure::Connection));
        }
        Ok(self.after.clone())
    }
    fn list_attempts(
        &mut self,
        _: &str,
        cursor: Option<String>,
    ) -> Result<Vec<RemoteAttempt>, PublishError> {
        self.calls.push("list_attempts");
        self.pages += 1;
        let start = cursor
            .map(|id| {
                self.history
                    .iter()
                    .position(|a| a.attempt_id == id)
                    .unwrap()
                    + 1
            })
            .unwrap_or(0);
        Ok(self.history.iter().skip(start).take(100).cloned().collect())
    }
    fn register_attempt(&mut self, input: &AttemptInput) -> Result<RemoteAttempt, PublishError> {
        self.calls.push("register_attempt");
        self.writes.push(input.clone());
        let response = self.write_response.clone().unwrap_or_else(|| remote(input));
        if self.store_on_write {
            self.after = Some(response.clone());
        }
        match self.error.take() {
            Some(error) => Err(error),
            None => Ok(response),
        }
    }
}

#[test]
fn verified_package_encodes_exact_contract_fields_and_confirms_registration() {
    use candid::types::{Label, TypeInner};
    let package = Package::new();
    let input = package.input();
    let decoded: AttemptInput = candid::decode_one(&candid::encode_one(&input).unwrap()).unwrap();
    assert_eq!(decoded, input);
    assert_eq!(decoded.mission_id, package.record.mission_id);
    assert_eq!(decoded.attempt_id, package.record.attempt_id);
    assert_eq!(decoded.criteria_hash, package.record.criteria_hash);
    assert_eq!(decoded.artifact_hash, package.record.artifact_hash);
    assert_eq!(decoded.report_hash, package.record.report_hash);
    assert_eq!(decoded.previous_attempt_id, None);
    assert_eq!(decoded.verdict, Verdict::PASS);
    assert_eq!(
        u128::from(decoded.timestamp_unix_ms),
        package.record.timestamp_unix_ms
    );
    let ty = AttemptInput::ty();
    let TypeInner::Record(fields) = ty.as_ref() else {
        panic!("expected record")
    };
    let mut names: Vec<_> = fields
        .iter()
        .map(|f| match f.id.as_ref() {
            Label::Named(name) => name.as_str(),
            _ => panic!("expected named field"),
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "artifact_hash",
            "attempt_id",
            "criteria_hash",
            "mission_id",
            "previous_attempt_id",
            "report_hash",
            "timestamp_unix_ms",
            "verdict"
        ]
    );
    assert!(fields.iter().any(
        |f| f.id.as_ref() == &Label::Named("timestamp_unix_ms".into())
            && matches!(f.ty.as_ref(), TypeInner::Nat64)
    ));
    let mut mock = Mock::new(&package);
    assert_eq!(package.publish(&mut mock).unwrap(), Outcome::Registered);
    assert_eq!(mock.writes, vec![input]);
    assert_eq!(
        mock.calls,
        [
            "get_mission",
            "get_attempt",
            "list_attempts",
            "register_attempt",
            "get_attempt"
        ]
    );
}

#[test]
fn modified_package_cannot_even_query_icp() {
    let package = Package::new();
    fs::write(
        package
            .root
            .join("attempts")
            .join(&package.record.attempt_id)
            .join("artifact.rs"),
        "tampered",
    )
    .unwrap();
    let mut mock = Mock::new(&package);
    assert!(package
        .publish(&mut mock)
        .unwrap_err()
        .to_string()
        .contains("SHA-256 mismatch"));
    assert!(mock.calls.is_empty());
}

#[test]
fn legacy_records_cannot_be_published_or_reconstructed() {
    let package = Package::new();
    let mut legacy = package.record.clone();
    legacy.evidence_format_version = None;
    fs::write(
        package.root.join("evidence.jsonl"),
        serde_json::to_vec(&legacy).unwrap(),
    )
    .unwrap();
    let mut mock = Mock::new(&package);
    assert!(package
        .publish(&mut mock)
        .unwrap_err()
        .to_string()
        .contains("histórico"));
    assert!(mock.calls.is_empty());
}

#[test]
fn missing_mission_and_mismatched_criteria_prevent_writes() {
    let package = Package::new();
    let mut mock = Mock::new(&package);
    mock.mission = None;
    assert!(matches!(
        package.publish(&mut mock),
        Err(PublishError::Registry(RegistryError::NotFound))
    ));
    assert!(mock.writes.is_empty());
    let mut mock = Mock::new(&package);
    mock.mission.as_mut().unwrap().criteria_hash = "b".repeat(64);
    assert!(package
        .publish(&mut mock)
        .unwrap_err()
        .to_string()
        .contains("criteria_hash"));
    assert!(mock.writes.is_empty());
}

#[test]
fn identical_duplicate_is_idempotent_even_after_remote_history_advances() {
    let package = Package::new();
    let mut mock = Mock::new(&package);
    mock.initial = Some(remote(&package.input()));
    assert_eq!(
        package.publish(&mut mock).unwrap(),
        Outcome::AlreadyRegistered
    );
    assert!(mock.writes.is_empty());
    assert_eq!(mock.calls, ["get_mission", "get_attempt"]);
}

#[test]
fn duplicate_changes_to_any_original_field_are_conflicts() {
    let package = Package::new();
    for field in 0..8 {
        let mut changed = package.input();
        match field {
            0 => changed.mission_id = "other".into(),
            1 => changed.attempt_id = "other".into(),
            2 => changed.previous_attempt_id = Some("other".into()),
            3 => changed.criteria_hash = "a".repeat(64),
            4 => changed.artifact_hash = "a".repeat(64),
            5 => changed.report_hash = "a".repeat(64),
            6 => changed.verdict = Verdict::FAIL,
            _ => changed.timestamp_unix_ms += 1,
        }
        let mut mock = Mock::new(&package);
        mock.initial = Some(remote(&changed));
        assert!(matches!(
            package.publish(&mut mock),
            Err(PublishError::Conflict(_))
        ));
        assert!(mock.writes.is_empty());
    }
}

#[test]
fn incorrect_predecessors_in_either_direction_prevent_writes() {
    let package = Package::new();
    let mut mock = Mock::new(&package);
    let mut old = package.input();
    old.attempt_id = "remote-tip".into();
    mock.history.push(remote(&old));
    assert!(matches!(
        package.publish(&mut mock),
        Err(PublishError::ChainConflict { .. })
    ));
    assert!(mock.writes.is_empty());
    let second = evidence::record_attempt(
        &package.root,
        &Evaluation::from_checks(Status::Pass, vec![]),
    )
    .unwrap();
    let mut mock = Mock::new(&package);
    assert!(matches!(
        publish(&package.root, &second.attempt_id, &mut mock),
        Err(PublishError::ChainConflict { expected: None, .. })
    ));
    assert!(mock.writes.is_empty());
}

#[test]
fn a_second_verified_package_extends_the_remote_chain_without_changing_its_predecessor() {
    let package = Package::new();
    let second = evidence::record_attempt(
        &package.root,
        &Evaluation::from_checks(Status::Pass, vec![]),
    )
    .unwrap();
    let mut mock = Mock::new(&package);
    mock.history.push(remote(&package.input()));
    assert_eq!(
        publish(&package.root, &second.attempt_id, &mut mock).unwrap(),
        Outcome::Registered
    );
    assert_eq!(mock.writes.len(), 1);
    assert_eq!(mock.writes[0], AttemptInput::from_record(second).unwrap());
    assert_eq!(
        mock.writes[0].previous_attempt_id.as_ref(),
        Some(&package.record.attempt_id)
    );
}

#[test]
fn chain_tip_uses_links_not_timestamp_order_and_paginates() {
    let package = Package::new();
    let mut mock = Mock::new(&package);
    let mut previous = None;
    for index in 0..205 {
        let mut input = package.input();
        input.attempt_id = format!("old-{index:03}");
        input.previous_attempt_id = previous;
        input.timestamp_unix_ms = 1000 - index;
        previous = Some(input.attempt_id.clone());
        mock.history.push(remote(&input));
    }
    mock.history.sort_by_key(|a| a.timestamp_unix_ms);
    assert_eq!(remote_tip(&mut mock, &package.input()).unwrap(), previous);
    assert_eq!(mock.pages, 3);
    let mut input = package.input();
    input.previous_attempt_id = previous;
    assert_eq!(
        remote_tip(&mut mock, &input).unwrap(),
        input.previous_attempt_id
    );
}

#[test]
fn broken_or_forked_remote_chains_are_rejected() {
    let package = Package::new();
    for mode in 0..4 {
        let mut mock = Mock::new(&package);
        let mut a = package.input();
        a.attempt_id = "a".into();
        let mut b = a.clone();
        b.attempt_id = "b".into();
        match mode {
            0 => a.previous_attempt_id = Some("absent".into()),
            1 => {
                a.previous_attempt_id = Some("b".into());
                b.previous_attempt_id = Some("a".into());
            }
            2 => {} // two roots
            _ => {
                b.previous_attempt_id = Some("a".into());
                let mut c = b.clone();
                c.attempt_id = "c".into();
                mock.history.push(remote(&c));
            }
        }
        mock.history.extend([remote(&a), remote(&b)]);
        assert!(matches!(
            remote_tip(&mut mock, &package.input()),
            Err(PublishError::InvalidResponse(_))
        ));
    }
}

#[test]
fn explicit_candid_rejections_including_chain_conflict_never_become_success() {
    let package = Package::new();
    for error in [
        RegistryError::Unauthorized,
        RegistryError::NotFound,
        RegistryError::ChainConflict {
            expected_previous_attempt_id: Some("racing-tip".into()),
        },
        RegistryError::InvalidInput("bad".into()),
        RegistryError::CapacityReached,
        RegistryError::StorageError("storage".into()),
    ] {
        let mut mock = Mock::new(&package);
        // Even if another writer made an identical ID visible, a definite Err is not Ok.
        mock.error = Some(PublishError::Registry(error.clone()));
        assert!(
            matches!(package.publish(&mut mock), Err(PublishError::Registry(actual)) if actual == error)
        );
        assert_eq!(
            mock.calls.iter().filter(|&&c| c == "get_attempt").count(),
            1
        );
    }
}

#[test]
fn already_exists_race_requires_an_identical_read_back() {
    let package = Package::new();
    let mut mock = Mock::new(&package);
    mock.error = Some(PublishError::Registry(RegistryError::AlreadyExists));
    assert_eq!(
        package.publish(&mut mock).unwrap(),
        Outcome::AlreadyRegistered
    );
    let mut mock = Mock::new(&package);
    let mut wrong = remote(&package.input());
    wrong.verdict = Verdict::ERROR;
    mock.write_response = Some(wrong);
    mock.error = Some(PublishError::Registry(RegistryError::AlreadyExists));
    assert!(matches!(
        package.publish(&mut mock),
        Err(PublishError::Conflict(_))
    ));
    let mut mock = Mock::new(&package);
    mock.store_on_write = false;
    mock.error = Some(PublishError::Registry(RegistryError::AlreadyExists));
    assert!(package.publish(&mut mock).is_err());
}

#[test]
fn ambiguous_network_results_are_reconciled_without_resubmission_or_false_success() {
    let package = Package::new();
    for stored in [false, true] {
        let mut mock = Mock::new(&package);
        mock.store_on_write = stored;
        mock.error = Some(PublishError::Cli(CliFailure::Connection));
        let result = package.publish(&mut mock);
        if stored {
            assert_eq!(result.unwrap(), Outcome::ConfirmedAfterUncertainResponse);
        } else {
            assert!(matches!(result, Err(PublishError::Unconfirmed(_))));
        }
        assert_eq!(mock.writes.len(), 1);
        assert_eq!(
            mock.calls.iter().filter(|&&c| c == "get_attempt").count(),
            2
        );
    }
    let mut mock = Mock::new(&package);
    mock.error = Some(PublishError::Cli(CliFailure::Connection));
    mock.fail_confirmation = true;
    assert!(matches!(
        package.publish(&mut mock),
        Err(PublishError::Unconfirmed(_))
    ));
}

#[test]
fn ok_update_still_requires_matching_response_and_final_query() {
    let package = Package::new();
    let mut mock = Mock::new(&package);
    mock.store_on_write = false;
    assert!(matches!(
        package.publish(&mut mock),
        Err(PublishError::Unconfirmed(_))
    ));
    let mut mock = Mock::new(&package);
    mock.fail_confirmation = true;
    assert!(matches!(
        package.publish(&mut mock),
        Err(PublishError::Unconfirmed(_))
    ));
    let mut mock = Mock::new(&package);
    let mut wrong = remote(&package.input());
    wrong.report_hash = "0".repeat(64);
    mock.write_response = Some(wrong);
    assert!(matches!(
        package.publish(&mut mock),
        Err(PublishError::Conflict(_))
    ));
}

#[test]
fn timestamps_are_checked_at_nat64_boundaries() {
    let package = Package::new();
    for timestamp in [0, u128::from(u64::MAX)] {
        let mut record = package.record.clone();
        record.timestamp_unix_ms = timestamp;
        assert_eq!(
            u128::from(AttemptInput::from_record(record).unwrap().timestamp_unix_ms),
            timestamp
        );
    }
    for timestamp in [u128::from(u64::MAX) + 1, u128::MAX] {
        let mut record = package.record.clone();
        record.timestamp_unix_ms = timestamp;
        assert!(AttemptInput::from_record(record)
            .unwrap_err()
            .to_string()
            .contains("nat64"));
    }
}

#[test]
fn all_verdicts_preserve_the_exact_candid_variants() {
    let package = Package::new();
    for (status, expected) in [
        (Status::Pass, Verdict::PASS),
        (Status::Fail, Verdict::FAIL),
        (Status::Error, Verdict::ERROR),
        (Status::NotRun, Verdict::NOT_RUN),
    ] {
        let mut record = package.record.clone();
        record.verdict = status;
        let input = AttemptInput::from_record(record).unwrap();
        let decoded: AttemptInput =
            candid::decode_one(&candid::encode_one(input).unwrap()).unwrap();
        assert_eq!(decoded.verdict, expected);
    }
}

#[test]
fn failed_publication_never_rewrites_local_index_or_packages() {
    let package = Package::new();
    let path = package.root.join("evidence.jsonl");
    let before = fs::read(&path).unwrap();
    let manifest = package
        .root
        .join("attempts")
        .join(&package.record.attempt_id)
        .join("manifest.json");
    let before_manifest = fs::read(&manifest).unwrap();
    let mut mock = Mock::new(&package);
    mock.error = Some(PublishError::Registry(RegistryError::Unauthorized));
    assert!(package.publish(&mut mock).is_err());
    assert_eq!(fs::read(path).unwrap(), before);
    assert_eq!(fs::read(manifest).unwrap(), before_manifest);
    assert!(matches!(
        evidence::verify_attempt(&package.root, &package.record.attempt_id),
        Ok(Verification::Verified(_))
    ));
}

#[test]
fn independent_demo_store_starts_a_new_chain_preserving_four_original_records() {
    let package = Package::new();
    for _ in 0..3 {
        evidence::record_attempt(
            &package.root,
            &Evaluation::from_checks(Status::Pass, vec![]),
        )
        .unwrap();
    }
    let original = fs::read(package.root.join("evidence.jsonl")).unwrap();
    assert_eq!(
        original
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .count(),
        4
    );
    let demo = package.root.join("demo-stage4");
    let first =
        evidence::record_attempt(&demo, &Evaluation::from_checks(Status::Pass, vec![])).unwrap();
    assert_eq!(first.previous_attempt_id, None);
    assert!(matches!(
        evidence::verify_attempt(&demo, &first.attempt_id),
        Ok(Verification::Verified(_))
    ));
    assert_eq!(
        sha256(&fs::read(package.root.join("evidence.jsonl")).unwrap()),
        sha256(&original)
    );
}

#[test]
fn oversized_timestamp_in_untrusted_package_is_rejected_without_panic_or_icp() {
    let package = Package::new();
    let mut record = package.record.clone();
    record.timestamp_unix_ms = u128::from(u64::MAX) + 1;
    let directory = package.root.join("attempts").join(&record.attempt_id);
    // Construct exact JSON bytes directly: serde_json::Value cannot hold this u128.
    let report = fs::read_to_string(directory.join("report.json"))
        .unwrap()
        .replace(
            &package.record.timestamp_unix_ms.to_string(),
            &record.timestamp_unix_ms.to_string(),
        );
    let payload = fs::read_to_string(directory.join("report-payload.json"))
        .unwrap()
        .replace(
            &package.record.timestamp_unix_ms.to_string(),
            &record.timestamp_unix_ms.to_string(),
        );
    record.report_hash = sha256(payload.as_bytes());
    let report = report.replace(&package.record.report_hash, &record.report_hash);
    fs::write(directory.join("report.json"), &report).unwrap();
    fs::write(directory.join("report-payload.json"), &payload).unwrap();
    let manifest = format!(
        "{{\"record\":{},\"report_file_hash\":\"{}\"}}",
        serde_json::to_string(&record).unwrap(),
        sha256(report.as_bytes())
    );
    fs::write(directory.join("manifest.json"), manifest).unwrap();
    fs::write(
        package.root.join("evidence.jsonl"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    let mut mock = Mock::new(&package);
    assert!(package
        .publish(&mut mock)
        .unwrap_err()
        .to_string()
        .contains("nat64"));
    assert!(mock.calls.is_empty());
}
