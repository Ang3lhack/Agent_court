use serde::Serialize;
use std::{
    fs::{self, File},
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};

use crate::process;

pub const AGENT_TIMEOUT: Duration = Duration::from_secs(180);
pub const AGENT_OUTPUT_LIMIT: usize = 256 * 1024;
const MESSAGE_LIMIT: u64 = 16 * 1024;
const PROMPT_LIMIT: usize = 384 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
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

    fn description(&self) -> &'static str {
        "SIMULATED AgentRunner (fixture; no Codex real)"
    }
}

#[derive(Default)]
pub struct CodexRunner;

impl AgentRunner for CodexRunner {
    fn run(&self, request: AgentRequest) -> Result<AgentResult, String> {
        run_codex(request, "codex".into(), AGENT_TIMEOUT)
    }

    fn description(&self) -> &'static str {
        "REAL Codex CLI (executable codex from PATH; not a bundled fixture)"
    }
}

struct RunFiles(PathBuf);
impl Drop for RunFiles {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_codex(
    request: AgentRequest,
    executable: PathBuf,
    timeout: Duration,
) -> Result<AgentResult, String> {
    if request.prompt.trim().is_empty() || request.prompt.len() > PROMPT_LIMIT {
        return Err("prompt must contain between 1 and 393,216 bytes".into());
    }
    if !request.working_dir.is_dir() {
        return Err(format!(
            "agent working directory does not exist: {}",
            request.working_dir.display()
        ));
    }
    let working_dir = fs::canonicalize(&request.working_dir).map_err(|e| e.to_string())?;
    let files = RunFiles(crate::temporary_workspace()?);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&files.0, fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    let prompt_path = files.0.join("prompt.txt");
    fs::write(&prompt_path, &request.prompt).map_err(|e| e.to_string())?;
    let last_message_path = files.0.join("last-message.txt");
    let mut command = Command::new(executable);
    command
        .arg("exec")
        .arg("--json")
        .arg("--sandbox")
        .arg(request.role.sandbox())
        .arg("--cd")
        .arg(&working_dir)
        .arg("--output-last-message")
        .arg(&last_message_path)
        .args([
            "--skip-git-repo-check",
            "--ephemeral",
            "--ignore-user-config",
            "--ignore-rules",
        ]);
    for config in [
        "approval_policy=\"never\"",
        "sandbox_workspace_write.writable_roots=[]",
        "sandbox_workspace_write.exclude_slash_tmp=true",
        "sandbox_workspace_write.exclude_tmpdir_env_var=true",
        "sandbox_workspace_write.network_access=false",
        "features.shell_tool=false",
        "features.multi_agent=false",
        "features.apps=false",
        "features.plugins=false",
        "features.hooks=false",
        "features.browser_use=false",
        "features.computer_use=false",
        "features.remote_plugin=false",
        "web_search=\"disabled\"",
    ] {
        command.args(["-c", config]);
    }
    // Feed large snapshots through stdin, avoiding OS per-argument limits.
    command
        .arg("-")
        .current_dir(&working_dir)
        .stdin(Stdio::from(
            File::open(&prompt_path).map_err(|e| e.to_string())?,
        ));
    let output = process::run_bounded(&mut command, timeout, AGENT_OUTPUT_LIMIT)
        .map_err(|error| format!("cannot run codex exec: {error}"))?;
    if output.stop != process::Stop::Completed {
        return Err(format!("codex exec stopped: {}", output.diagnostic()));
    }

    let mut message = Vec::new();
    if last_message_path.exists() {
        if !fs::symlink_metadata(&last_message_path)
            .map_err(|e| e.to_string())?
            .is_file()
        {
            return Err("codex last message is not a regular file".into());
        }
        File::open(&last_message_path)
            .and_then(|f| f.take(MESSAGE_LIMIT + 1).read_to_end(&mut message))
            .map_err(|e| format!("cannot read codex last message: {e}"))?;
    }
    if message.len() as u64 > MESSAGE_LIMIT {
        return Err("codex last message exceeds 16 KiB".into());
    }
    Ok(AgentResult {
        role: request.role,
        success: output.success(),
        exit_code: output.code,
        last_message: String::from_utf8(message).map_err(|_| "codex message is not UTF-8")?,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

#[cfg(test)]
mod tests;
