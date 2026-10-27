use serde::Serialize;
use std::{
    env, fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Builder,
    Reviewer,
    Repair,
}
impl AgentRole {
    fn sandbox(self) -> &'static str {
        match self {
            Self::Reviewer => "read-only",
            Self::Builder | Self::Repair => "workspace-write",
        }
    }
}

#[derive(Debug)]
pub struct AgentRequest {
    pub role: AgentRole,
    pub prompt: String,
    pub working_dir: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct AgentResult {
    pub role: AgentRole,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub last_message: String,
    pub stdout: String,
    pub stderr: String,
}

pub trait AgentRunner {
    fn run(&self, request: AgentRequest) -> Result<AgentResult, String>;
}

#[derive(Default)]
pub struct CodexRunner;

impl AgentRunner for CodexRunner {
    fn run(&self, request: AgentRequest) -> Result<AgentResult, String> {
        if request.prompt.trim().is_empty() || request.prompt.len() > 32_000 {
            return Err("prompt must contain between 1 and 32,000 bytes".into());
        }
        if !request.working_dir.is_dir() {
            return Err(format!(
                "agent working directory does not exist: {}",
                request.working_dir.display()
            ));
        }

        let last_message_path = temporary_message_path()?;
        let output = Command::new("codex")
            .arg("exec")
            .arg("--json")
            .arg("--sandbox")
            .arg(request.role.sandbox())
            .arg("--cd")
            .arg(&request.working_dir)
            .arg("--output-last-message")
            .arg(&last_message_path)
            .arg(&request.prompt)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("cannot start codex exec: {error}"))?;

        let last_message = fs::read_to_string(&last_message_path).unwrap_or_default();
        let _ = fs::remove_file(last_message_path);

        Ok(AgentResult {
            role: request.role,
            success: output.status.success(),
            exit_code: output.status.code(),
            last_message,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

fn temporary_message_path() -> Result<PathBuf, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock error: {error}"))?
        .as_nanos();
    Ok(env::temp_dir().join(format!(
        "agent-court-codex-{}-{nonce}.txt",
        std::process::id()
    )))
}
