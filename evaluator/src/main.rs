use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    time::{SystemTime, UNIX_EPOCH},
};

mod evidence;
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[allow(dead_code)]
pub enum Status {
    Pass,
    Fail,
    Error,
    NotRun,
}

#[derive(Debug, Serialize)]
struct Check {
    name: &'static str,
    status: Status,
    detail: String,
}

#[derive(Debug, Serialize)]
pub struct EvaluationReport {
    schema_version: u32,
    mission_id: &'static str,
    pub verdict: Status,
    checks: Vec<Check>,
    pub criteria_hash: String,
    pub artifact_hash: String,
    pub report_hash: String,
    timestamp_unix_ms: u128,
    isolation: &'static str,
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

fn evaluate() -> EvaluationReport {
    let artifact_hash = sha256(TEST_SOURCE.as_bytes());
    let criteria_hash = sha256(CRITERIA.as_bytes());
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
                name: "counter_contract",
                status: Status::Pass,
                detail: "3 Rust tests passed: initial value, increment, reset".into(),
            });
        }
        Ok(status) => {
            verdict = Status::Fail;
            checks.push(Check {
                name: "counter_contract",
                status: Status::Fail,
                detail: format!("Rust test process exited with {status}"),
            });
        }
        Err(error) => checks.push(Check {
            name: "counter_contract",
            status: Status::Error,
            detail: error,
        }),
    }

    let timestamp_unix_ms = timestamp_ms();
    let hash_input = serde_json::json!({
        "schema_version": 1,
        "mission_id": MISSION_ID,
        "verdict": verdict,
        "checks": checks,
        "criteria_hash": criteria_hash,
        "artifact_hash": artifact_hash,
        "timestamp_unix_ms": timestamp_unix_ms,
    });
    let report_hash = sha256(hash_input.to_string().as_bytes());

    EvaluationReport {
        schema_version: 1,
        mission_id: MISSION_ID,
        verdict,
        checks,
        criteria_hash,
        artifact_hash,
        report_hash,
        timestamp_unix_ms,
        isolation: "unique temporary directory; trusted built-in fixture only",
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().is_some_and(|command| command == "agent") {
        run_agent_command(&args[1..]);
        return;
    }

    let report = evaluate();
    let evidence_path = PathBuf::from(".agent-court/evidence.jsonl");
    match evidence::record_attempt(&evidence_path, &report) {
        Ok(record) => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "report": report,
                "evidence": record,
                "evidence_path": evidence_path,
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
            eprintln!("unknown role: {}; expected builder, reviewer, or repair", args[0]);
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
            println!("{}", serde_json::to_string_pretty(&result).expect("result serializes"));
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
        let report = evaluate();
        assert!(matches!(report.verdict, Status::Pass));
        assert_eq!(report.checks.len(), 1);
        assert!(report
            .checks
            .iter()
            .all(|check| matches!(check.status, Status::Pass)));
        assert_eq!(report.artifact_hash, sha256(TEST_SOURCE.as_bytes()));
        assert_eq!(report.report_hash.len(), 64);
    }
}
