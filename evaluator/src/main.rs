use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    time::{SystemTime, UNIX_EPOCH},
};

mod candidate;
mod cli;
mod evidence;
mod orchestrator;
mod process;
mod publisher;
mod runner;

const MISSION_ID: &str = "counter-v1";
const CRITERIA: &str = "counter-v1: starts at zero; increment increases by one; reset returns zero";
const TEST_SOURCE: &str = r#"
#[derive(Default)]
struct Counter(u64);

impl Counter {
    fn increment(&mut self) { self.0 += 1; }
    fn value(&self) -> u64 { self.0 }
    fn reset(&mut self) { self.0 = 0; }
}

#[test]
fn starts_at_zero() { assert_eq!(Counter::default().value(), 0); }

#[test]
fn increment_advances_value() {
    let mut counter = Counter::default();
    counter.increment();
    assert_eq!(counter.value(), 1);
}

#[test]
fn reset_clears_value() {
    let mut counter = Counter::default();
    counter.increment();
    counter.reset();
    assert_eq!(counter.value(), 0);
}
"#;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[allow(dead_code)]
pub enum Status {
    Pass,
    Fail,
    Error,
    NotRun,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Check {
    name: String,
    status: Status,
    detail: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EvaluationReport {
    schema_version: u32,
    mission_id: String,
    pub verdict: Status,
    checks: Vec<Check>,
    pub criteria_hash: String,
    pub artifact_hash: String,
    pub report_hash: String,
    timestamp_unix_ms: u128,
    isolation: String,
}

const REPORT_SCHEMA_VERSION: u32 = 2;

impl EvaluationReport {
    // This payload includes every report field except its own SHA-256 digest.
    fn hash_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "schema_version": self.schema_version,
            "mission_id": self.mission_id,
            "verdict": self.verdict,
            "checks": self.checks,
            "criteria_hash": self.criteria_hash,
            "artifact_hash": self.artifact_hash,
            "timestamp_unix_ms": self.timestamp_unix_ms,
            "isolation": self.isolation,
        })
    }
}

pub struct Evaluation {
    report: EvaluationReport,
    criteria: Vec<u8>,
    artifact: Vec<u8>,
    report_payload: Vec<u8>,
}

impl Evaluation {
    fn from_checks(verdict: Status, checks: Vec<Check>) -> Self {
        Self::from_artifact(
            verdict,
            checks,
            TEST_SOURCE.as_bytes().to_vec(),
            "unique temporary directory; trusted built-in fixture only".into(),
        )
    }

    fn from_artifact(
        verdict: Status,
        checks: Vec<Check>,
        artifact: Vec<u8>,
        isolation: String,
    ) -> Self {
        let criteria = CRITERIA.as_bytes().to_vec();
        let mut report = EvaluationReport {
            schema_version: REPORT_SCHEMA_VERSION,
            mission_id: MISSION_ID.into(),
            verdict,
            checks,
            criteria_hash: sha256(&criteria),
            artifact_hash: sha256(&artifact),
            report_hash: String::new(),
            timestamp_unix_ms: timestamp_ms(),
            isolation,
        };
        let report_payload =
            serde_json::to_vec(&report.hash_payload()).expect("payload serializes");
        report.report_hash = sha256(&report_payload);
        Self {
            report,
            criteria,
            artifact,
            report_payload,
        }
    }
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn temporary_workspace() -> Result<PathBuf, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock error: {error}"))?
        .as_nanos();
    let path = env::temp_dir().join(format!("agent-court-eval-{}-{now}", std::process::id()));
    fs::create_dir(&path)
        .map_err(|error| format!("cannot create evaluation workspace: {error}"))?;
    Ok(path)
}

fn run_test_binary(source: &Path, output_dir: &Path) -> Result<ExitStatus, String> {
    let binary_name = if cfg!(windows) {
        "counter_tests.exe"
    } else {
        "counter_tests"
    };
    let binary = output_dir.join(binary_name);
    let compilation = Command::new("rustc")
        .arg("--edition=2021")
        .arg("--test")
        .arg(source)
        .arg("-o")
        .arg(&binary)
        .output()
        .map_err(|error| format!("cannot start rustc: {error}"))?;

    if !compilation.status.success() {
        return Err(format!(
            "counter test compilation failed ({}): {}",
            compilation.status,
            String::from_utf8_lossy(&compilation.stderr).trim()
        ));
    }

    Command::new(binary)
        .status()
        .map_err(|error| format!("cannot run counter test binary: {error}"))
}

fn timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn evaluate() -> Evaluation {
    let mut checks = Vec::new();
    let mut verdict = Status::Error;
    let result = (|| -> Result<ExitStatus, String> {
        let workspace = temporary_workspace()?;
        let source = workspace.join("counter_tests.rs");
        fs::write(&source, TEST_SOURCE)
            .map_err(|error| format!("cannot write test source: {error}"))?;
        let result = run_test_binary(&source, &workspace);
        // Remove only the unique workspace created by this invocation.
        let _ = fs::remove_dir_all(workspace);
        result
    })();

    match result {
        Ok(status) if status.success() => {
            verdict = Status::Pass;
            checks.push(Check {
                name: "counter_contract".into(),
                status: Status::Pass,
                detail: "3 Rust tests passed: initial value, increment, reset".into(),
            });
        }
        Ok(status) => {
            verdict = Status::Fail;
            checks.push(Check {
                name: "counter_contract".into(),
                status: Status::Fail,
                detail: format!("Rust test process exited with {status}"),
            });
        }
        Err(error) => checks.push(Check {
            name: "counter_contract".into(),
            status: Status::Error,
            detail: error,
        }),
    }

    Evaluation::from_checks(verdict, checks)
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let cli = cli::Cli::parse(&args).unwrap_or_else(|usage| {
        eprintln!("{usage}");
        std::process::exit(64);
    });
    let evidence_root = cli.evidence_dir;
    match cli.action {
        cli::Action::Evaluate => {}
        cli::Action::Orchestrate {
            publish,
            identity,
            demo_seed,
        } => {
            let mut publish_attempt = |root: &Path, id: &str| {
                publisher::IcpCli::new(&identity)
                    .and_then(|mut registry| publisher::publish(root, id, &mut registry))
                    .map(|outcome| outcome.to_string())
                    .map_err(|error| error.to_string())
            };
            let publication = if publish {
                Some(&mut publish_attempt as &mut orchestrator::PublishFn<'_>)
            } else {
                None
            };
            let code = match orchestrator::orchestrate(
                &evidence_root,
                &runner::CodexRunner,
                demo_seed.as_deref(),
                publication,
                &mut |event| println!("{event}"),
            ) {
                Ok(outcome) => {
                    println!(
                        "MISSION END verdict={} evaluations={} repaired={}",
                        orchestrator::label(outcome.verdict),
                        outcome.attempts.len(),
                        outcome.repaired
                    );
                    match outcome.verdict {
                        Status::Pass => 0,
                        Status::Fail => 1,
                        Status::Error => 2,
                        Status::NotRun => 3,
                    }
                }
                Err(error) => {
                    eprintln!("ORCHESTRATE ERROR: {error}");
                    2
                }
            };
            std::process::exit(code);
        }
        cli::Action::EvaluateFile { path, no_run } => {
            match candidate::evaluate_file(&path, no_run) {
                Ok(evaluation) => {
                    let code = match evaluation.report.verdict {
                        Status::Pass => 0,
                        Status::Fail => 1,
                        Status::Error => 2,
                        Status::NotRun => 3,
                    };
                    record_evaluation(&evidence_root, evaluation);
                    std::process::exit(code);
                }
                Err(error) => {
                    eprintln!("ERROR: {error}; no se creó un intento sin los bytes del candidato");
                    std::process::exit(2);
                }
            }
        }
        cli::Action::Agent(rest) => {
            run_agent_command(&rest);
            return;
        }
        cli::Action::Verify(attempt_id) => {
            run_verify_command(&evidence_root, &attempt_id);
            return;
        }
        cli::Action::Publish {
            attempt_id,
            identity,
        } => {
            let result = publisher::IcpCli::new(&identity).and_then(|mut registry| {
                publisher::publish(&evidence_root, &attempt_id, &mut registry)
            });
            match result {
                Ok(outcome) => println!("{outcome}: {attempt_id} (backend, entorno local). Metadatos originales confirmados mediante get_attempt."),
                Err(error) => {
                    eprintln!("PUBLICACIÓN FALLIDA: {error}");
                    std::process::exit(2);
                }
            }
            return;
        }
        cli::Action::Help => {
            println!("{}", cli::USAGE);
            return;
        }
    }

    let evaluation = evaluate();
    record_evaluation(&evidence_root, evaluation);
}

fn record_evaluation(evidence_root: &Path, evaluation: Evaluation) {
    match evidence::record_attempt(evidence_root, &evaluation) {
        Ok(record) => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "report": evaluation.report,
                "evidence": record,
                "evidence_path": evidence_root.join("evidence.jsonl"),
                "package_path": evidence_root.join("attempts").join(&record.attempt_id),
            }))
            .expect("result serializes")
        ),
        Err(error) => {
            eprintln!(
                "evidence persistence failed; evaluation is not considered complete: {error}"
            );
            std::process::exit(2);
        }
    }
}

fn run_verify_command(evidence_root: &Path, attempt_id: &str) {
    match evidence::verify_attempt(evidence_root, attempt_id) {
        Ok(evidence::Verification::Verified(record)) => {
            println!("VERIFICADO: {}\ncriteria.txt: {}\nartifact.rs: {}\nreport-payload.json: {}\nEl informe, el manifiesto y el registro JSONL son consistentes.", record.attempt_id, record.criteria_hash, record.artifact_hash, record.report_hash);
        }
        Ok(evidence::Verification::Legacy(record)) => {
            eprintln!("NO VERIFICABLE: {} es un registro histórico de solo metadatos. No se conservaron sus archivos originales; no se han reconstruido ni inventado evidencias.", record.attempt_id);
            std::process::exit(3);
        }
        Err(error) => {
            eprintln!("VERIFICACIÓN FALLIDA: {error}");
            std::process::exit(2);
        }
    }
}

fn run_agent_command(args: &[String]) {
    use runner::{AgentRequest, AgentRole, AgentRunner, CodexRunner};

    if args.len() < 3 {
        eprintln!("usage: agent <builder|reviewer|repair> <working-directory> <prompt>");
        std::process::exit(64);
    }
    let role = match args[0].as_str() {
        "builder" => AgentRole::Builder,
        "reviewer" => AgentRole::Reviewer,
        "repair" => AgentRole::Repair,
        _ => {
            eprintln!(
                "unknown role: {}; expected builder, reviewer, or repair",
                args[0]
            );
            std::process::exit(64);
        }
    };
    let request = AgentRequest {
        role,
        working_dir: PathBuf::from(&args[1]),
        prompt: args[2..].join(" "),
    };
    match CodexRunner.run(request) {
        Ok(result) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&result).expect("result serializes")
            );
            if !result.success {
                std::process::exit(result.exit_code.unwrap_or(1));
            }
        }
        Err(error) => {
            eprintln!("agent execution failed: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_uses_standard_digest() {
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn counter_mission_executes_in_a_temporary_workspace() {
        let evaluation = evaluate();
        let report = evaluation.report;
        assert!(matches!(report.verdict, Status::Pass));
        assert_eq!(report.checks.len(), 1);
        assert!(report
            .checks
            .iter()
            .all(|check| matches!(check.status, Status::Pass)));
        assert_eq!(report.artifact_hash, sha256(TEST_SOURCE.as_bytes()));
        assert_eq!(report.report_hash.len(), 64);
        assert_eq!(report.report_hash, sha256(&evaluation.report_payload));
    }
}
