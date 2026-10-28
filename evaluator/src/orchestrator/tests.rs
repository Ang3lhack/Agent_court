use super::*;
use crate::{runner::AgentResult, Check};
use std::cell::RefCell;

const GOOD: &str = include_str!("../../tests/fixtures/counter-pass.rs");
const BAD: &str = include_str!("../../tests/fixtures/counter-fail.rs");

struct TestStore(PathBuf);
impl TestStore {
    fn new() -> Self {
        Self(crate::temporary_workspace().unwrap())
    }
    fn store(&self) -> PathBuf {
        self.0.join("independent-evidence")
    }
}
impl Drop for TestStore {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct FixtureRunner {
    builder: &'static str,
    repair: &'static str,
    calls: RefCell<Vec<AgentRequest>>,
    fail: Option<AgentRole>,
    extra_file: bool,
    tamper_review: bool,
    empty_review: bool,
}
impl FixtureRunner {
    fn new(builder: &'static str, repair: &'static str) -> Self {
        Self {
            builder,
            repair,
            calls: RefCell::new(Vec::new()),
            fail: None,
            extra_file: false,
            tamper_review: false,
            empty_review: false,
        }
    }
    fn roles(&self) -> Vec<AgentRole> {
        self.calls.borrow().iter().map(|r| r.role).collect()
    }
}
impl AgentRunner for FixtureRunner {
    fn run(&self, request: AgentRequest) -> Result<AgentResult, String> {
        let role = request.role;
        let path = request.working_dir.join("candidate.rs");
        assert!(!request.working_dir.starts_with(env!("CARGO_MANIFEST_DIR")));
        assert_eq!(fs::read_dir(&request.working_dir).unwrap().count(), 1);
        match role {
            AgentRole::Builder => fs::write(&path, self.builder).unwrap(),
            AgentRole::Reviewer => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o222, 0);
                    assert_eq!(
                        fs::metadata(&request.working_dir)
                            .unwrap()
                            .permissions()
                            .mode()
                            & 0o222,
                        0
                    );
                }
                assert_eq!(fs::read_to_string(&path).unwrap(), self.builder);
                assert!(request.prompt.contains("REAL EVALUATOR DIAGNOSTICS"));
                assert!(request
                    .prompt
                    .contains(self.builder.lines().next().unwrap()));
                if self.tamper_review {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
                    }
                    fs::write(&path, GOOD).unwrap();
                }
            }
            AgentRole::Repair => {
                assert_eq!(fs::read_to_string(&path).unwrap(), self.builder);
                assert!(request
                    .prompt
                    .contains("fixture review: increment is wrong"));
                assert!(request.prompt.contains("REAL EVALUATOR DIAGNOSTICS"));
                fs::write(&path, self.repair).unwrap();
            }
        }
        if self.extra_file {
            fs::write(request.working_dir.join("extra.rs"), "forbidden").unwrap();
        }
        self.calls.borrow_mut().push(request);
        Ok(AgentResult {
            role,
            success: self.fail != Some(role),
            exit_code: Some(if self.fail == Some(role) { 7 } else { 0 }),
            last_message: if self.empty_review && role == AgentRole::Reviewer {
                String::new()
            } else {
                "fixture review: increment is wrong".into()
            },
            stdout: String::new(),
            stderr: "controlled diagnostic".into(),
        })
    }
}

// This evaluator is explicitly simulated. Real code execution is covered by
// docker_orchestrated_pass_and_repair below, never by host rustc or a model claim.
fn simulated(path: &Path, status: Status) -> Result<Evaluation, String> {
    Ok(Evaluation::from_artifact(
        status,
        vec![Check {
            name: "simulated_check".into(),
            status,
            detail: "SIMULATED evaluator diagnostic: increment expected 1, got 2".into(),
        }],
        candidate::read_candidate(path)?,
        "SIMULATED evaluator; no candidate execution".into(),
    ))
}

fn records(store: &Path) -> Vec<EvidenceRecord> {
    fs::read_to_string(store.join("evidence.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn assert_verified(store: &Path, outcome: &Outcome) {
    for record in &outcome.attempts {
        assert!(matches!(
            evidence::verify_attempt(store, &record.attempt_id),
            Ok(Verification::Verified(_))
        ));
    }
}

#[test]
fn first_pass_runs_only_builder_and_one_evaluation_without_publication() {
    let root = TestStore::new();
    let runner = FixtureRunner::new(GOOD, BAD);
    let mut events = vec![];
    let outcome = run_with(
        &root.store(),
        &runner,
        |path| simulated(path, Status::Pass),
        None,
        &mut |e| events.push(e),
    )
    .unwrap();
    assert_eq!(outcome.verdict, Status::Pass);
    assert!(!outcome.repaired);
    assert_eq!(runner.roles(), [AgentRole::Builder]);
    assert_eq!(outcome.attempts.len(), 1);
    assert_eq!(outcome.attempts[0].previous_attempt_id, None);
    assert!(events.iter().any(|e| e.contains("SIMULATED AgentRunner")));
    assert!(events.iter().any(|e| e.contains("REPAIR no fue necesario")));
    assert_verified(&root.store(), &outcome);
    assert!(runner
        .calls
        .borrow()
        .iter()
        .all(|r| !r.working_dir.exists()));
}

#[test]
fn fail_review_repair_pass_preserves_evaluated_bytes_and_existing_chain() {
    let root = TestStore::new();
    let seed = evidence::record_attempt(
        &root.store(),
        &simulated(
            &{
                let path = root.0.join("seed.rs");
                fs::write(&path, GOOD).unwrap();
                path
            },
            Status::NotRun,
        )
        .unwrap(),
    )
    .unwrap();
    let old_log = fs::read(root.store().join("evidence.jsonl")).unwrap();
    let old_manifest = fs::read(
        root.store()
            .join("attempts")
            .join(&seed.attempt_id)
            .join("manifest.json"),
    )
    .unwrap();
    let runner = FixtureRunner::new(BAD, GOOD);
    let outcome = run_with(
        &root.store(),
        &runner,
        |path| {
            simulated(
                path,
                if fs::read_to_string(path).unwrap() == BAD {
                    Status::Fail
                } else {
                    Status::Pass
                },
            )
        },
        None,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(
        runner.roles(),
        [AgentRole::Builder, AgentRole::Reviewer, AgentRole::Repair]
    );
    assert_eq!(outcome.verdict, Status::Pass);
    assert!(outcome.repaired);
    assert_eq!(outcome.attempts.len(), 2);
    assert_eq!(
        outcome.attempts[0].previous_attempt_id,
        Some(seed.attempt_id.clone())
    );
    assert_eq!(
        outcome.attempts[1].previous_attempt_id.as_ref(),
        Some(&outcome.attempts[0].attempt_id)
    );
    for (record, bytes) in outcome.attempts.iter().zip([BAD, GOOD]) {
        assert_eq!(
            fs::read(
                root.store()
                    .join("attempts")
                    .join(&record.attempt_id)
                    .join("artifact.rs")
            )
            .unwrap(),
            bytes.as_bytes()
        );
    }
    assert!(fs::read(root.store().join("evidence.jsonl"))
        .unwrap()
        .starts_with(&old_log));
    assert_eq!(
        fs::read(
            root.store()
                .join("attempts")
                .join(&seed.attempt_id)
                .join("manifest.json")
        )
        .unwrap(),
        old_manifest
    );
    let calls = runner.calls.borrow();
    assert_ne!(calls[0].working_dir, calls[1].working_dir);
    assert_ne!(calls[1].working_dir, calls[2].working_dir);
    assert!(calls[1].prompt.contains("increment expected 1, got 2"));
    assert!(calls[2].prompt.contains("increment expected 1, got 2"));
    assert_verified(&root.store(), &outcome);
}

#[test]
fn repeated_fail_stops_after_exactly_one_repair_and_two_evaluations() {
    let root = TestStore::new();
    let runner = FixtureRunner::new(BAD, BAD);
    let outcome = run_with(
        &root.store(),
        &runner,
        |path| simulated(path, Status::Fail),
        None,
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(outcome.verdict, Status::Fail);
    assert_eq!(outcome.attempts.len(), 2);
    assert_eq!(
        runner.roles(),
        [AgentRole::Builder, AgentRole::Reviewer, AgentRole::Repair]
    );
    assert_eq!(records(&root.store()).len(), 2);
    assert_verified(&root.store(), &outcome);
}

#[test]
fn infrastructure_error_and_not_run_preserve_verdict_and_never_trigger_repair() {
    for status in [Status::Error, Status::NotRun] {
        let root = TestStore::new();
        let runner = FixtureRunner::new(GOOD, GOOD);
        let outcome = run_with(
            &root.store(),
            &runner,
            |path| {
                if status == Status::NotRun {
                    candidate::evaluate_file(path, true)
                } else {
                    simulated(path, status)
                }
            },
            None,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(outcome.verdict, status);
        assert!(!outcome.repaired);
        assert_eq!(runner.roles(), [AgentRole::Builder]);
        assert_eq!(outcome.attempts.len(), 1);
        assert_verified(&root.store(), &outcome);
    }
}

#[test]
fn infrastructure_failure_after_repair_never_becomes_pass_or_a_third_evaluation() {
    for status in [Status::Error, Status::NotRun] {
        let root = TestStore::new();
        let runner = FixtureRunner::new(BAD, GOOD);
        let mut evaluations = 0;
        let outcome = run_with(
            &root.store(),
            &runner,
            |path| {
                evaluations += 1;
                simulated(
                    path,
                    if evaluations == 1 {
                        Status::Fail
                    } else {
                        status
                    },
                )
            },
            None,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(outcome.verdict, status);
        assert_eq!(evaluations, 2);
        assert_eq!(outcome.attempts.len(), 2);
        assert_eq!(
            runner.roles(),
            [AgentRole::Builder, AgentRole::Reviewer, AgentRole::Repair]
        );
        assert_verified(&root.store(), &outcome);
    }
}

#[test]
fn builder_success_claim_without_source_cannot_trigger_evaluation() {
    let root = TestStore::new();
    let runner = FixtureRunner::new("", GOOD);
    assert!(run_with(
        &root.store(),
        &runner,
        |_| panic!("must not evaluate"),
        None,
        &mut |_| {}
    )
    .err()
    .unwrap()
    .contains("no vacío"));
    assert!(!root.store().exists());
}

#[test]
fn agent_workspace_cannot_be_inside_the_selected_evidence_root() {
    assert!(Workspace::new(&std::env::temp_dir(), GOOD.as_bytes())
        .err()
        .unwrap()
        .contains("fuera del repositorio y del almacén"));
}

#[test]
fn agent_failure_empty_review_or_unexpected_writes_cannot_be_success() {
    for role in [AgentRole::Builder, AgentRole::Reviewer, AgentRole::Repair] {
        let root = TestStore::new();
        let mut runner = FixtureRunner::new(BAD, GOOD);
        runner.fail = Some(role);
        assert!(run_with(
            &root.store(),
            &runner,
            |path| simulated(path, Status::Fail),
            None,
            &mut |_| {}
        )
        .is_err());
        if role == AgentRole::Builder {
            assert!(!root.store().exists());
        } else {
            assert_eq!(records(&root.store()).len(), 1);
        }
    }
    for variant in 0..3 {
        let root = TestStore::new();
        let mut runner = FixtureRunner::new(BAD, GOOD);
        runner.extra_file = variant == 0;
        runner.tamper_review = variant == 1;
        runner.empty_review = variant == 2;
        assert!(run_with(
            &root.store(),
            &runner,
            |path| simulated(path, Status::Fail),
            None,
            &mut |_| {}
        )
        .is_err());
        assert!(!runner.roles().contains(&AgentRole::Repair));
    }
}

#[test]
fn explicit_publication_receives_only_new_verified_attempts_in_order_and_stops_on_conflict() {
    for conflict in [false, true] {
        let root = TestStore::new();
        let runner = FixtureRunner::new(BAD, GOOD);
        let mut published = vec![];
        let mut publish = |store: &Path, id: &str| {
            let Verification::Verified(record) = evidence::verify_attempt(store, id)? else {
                panic!("must verify")
            };
            let before = fs::read(store.join("evidence.jsonl")).unwrap();
            published.push(record);
            assert_eq!(fs::read(store.join("evidence.jsonl")).unwrap(), before);
            if conflict {
                Err("ChainConflict: controlled remote predecessor mismatch".into())
            } else {
                Ok("SIMULATED publication".into())
            }
        };
        let mut events = vec![];
        let outcome = run_with(
            &root.store(),
            &runner,
            |path| {
                simulated(
                    path,
                    if fs::read_to_string(path).unwrap() == BAD {
                        Status::Fail
                    } else {
                        Status::Pass
                    },
                )
            },
            Some(&mut publish),
            &mut |e| events.push(e),
        );
        if conflict {
            assert!(outcome.err().unwrap().contains("ChainConflict"));
            assert_eq!(published.len(), 1);
            assert_eq!(runner.roles(), [AgentRole::Builder]);
            assert!(events.iter().any(|e| e.contains("PUBLICATION ERROR")));
        } else {
            let outcome = outcome.unwrap();
            assert_eq!(published, outcome.attempts);
            assert_eq!(
                published[1].previous_attempt_id.as_ref(),
                Some(&published[0].attempt_id)
            );
        }
    }
}

/// Simulated agents, REAL Docker evaluation/diagnostics/evidence. No Codex or ICP.
#[test]
#[ignore = "requires Docker and the pre-pulled Rust image; agents remain simulated"]
fn docker_orchestrated_pass_and_repair() {
    for source in [GOOD, BAD] {
        let root = TestStore::new();
        let runner = FixtureRunner::new(source, GOOD);
        let outcome = orchestrate(&root.store(), &runner, None, &mut |e| println!("{e}")).unwrap();
        assert_eq!(outcome.verdict, Status::Pass);
        assert_eq!(outcome.repaired, source == BAD);
        assert_eq!(outcome.attempts.len(), if source == BAD { 2 } else { 1 });
        assert_verified(&root.store(), &outcome);
        if source == BAD {
            assert_eq!(outcome.attempts[0].verdict, Status::Fail);
            let calls = runner.calls.borrow();
            assert!(calls[1].prompt.contains("assertion `left == right` failed"));
            assert!(calls[2].prompt.contains("assertion `left == right` failed"));
        }
    }
}
