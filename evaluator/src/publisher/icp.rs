//! icp-cli 1.7 transport: binary Candid in/out, no shell and no secret handling.
use super::*;
use serde::de::DeserializeOwned;
use std::{
    io::IsTerminal,
    path::PathBuf,
    process::{Command, Stdio},
};

pub struct IcpCli {
    executable: PathBuf,
    project_root: PathBuf,
    identity: String,
    local_checked: bool,
    interactive: bool,
}

impl IcpCli {
    pub fn new(identity: &str) -> Result<Self, PublishError> {
        if !evidence::valid_id(identity) {
            return Err(PublishError::Validation(
                "nombre de identidad inválido".into(),
            ));
        }
        Ok(Self {
            executable: "icp".into(),
            project_root: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("evaluator has a project parent")
                .to_path_buf(),
            identity: identity.into(),
            local_checked: false,
            interactive: std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
        })
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        command
            .current_dir(&self.project_root)
            .env_remove("ICP_NETWORK")
            .env_remove("ICP_ENVIRONMENT")
            .env_remove("ICP_PROJECT_ROOT")
            .stdin(Stdio::inherit());
        command
    }

    fn ensure_local(&mut self) -> Result<(), PublishError> {
        if self.local_checked {
            return Ok(());
        }
        let mut command = self.command();
        command
            .args([
                "network",
                "status",
                "--environment",
                "local",
                "--json",
                "--project-root-override",
            ])
            .arg(&self.project_root);
        let bytes = execute(&mut command, false)?;
        #[derive(Deserialize)]
        struct Network {
            managed: bool,
            api_url: String,
        }
        let network: Network = serde_json::from_slice(&bytes).map_err(|_| {
            PublishError::InvalidResponse("network status no devolvió el JSON esperado".into())
        })?;
        // No configurable public target in this MVP, including via inherited env.
        let local_url = network
            .api_url
            .strip_prefix("http://127.0.0.1:")
            .or_else(|| network.api_url.strip_prefix("http://localhost:"))
            .or_else(|| network.api_url.strip_prefix("http://[::1]:"));
        let port = local_url.and_then(|s| s.strip_suffix('/').unwrap_or(s).parse::<u16>().ok());
        if !network.managed || !port.is_some_and(|port| port != 0) {
            return Err(PublishError::Validation("publish requiere una red local administrada en loopback; destino público rechazado".into()));
        }
        self.local_checked = true;
        Ok(())
    }

    fn call_command(&self, method: &str, args: Vec<u8>, query: bool) -> Command {
        let mut command = self.command();
        command
            .args([
                "canister",
                "call",
                "backend",
                method,
                &hex::encode(args),
                "--args-format",
                "hex",
                "--output",
                "hex",
                "--environment",
                "local",
                "--identity",
                if query { "anonymous" } else { &self.identity },
                "--candid",
            ])
            .arg(self.project_root.join("backend/backend.did"))
            .arg("--project-root-override")
            .arg(&self.project_root);
        if query {
            command.arg("--query");
        }
        command
    }

    fn call<T: CandidType + DeserializeOwned>(
        &mut self,
        method: &str,
        args: Result<Vec<u8>, candid::Error>,
        query: bool,
    ) -> Result<T, PublishError> {
        let args = args.map_err(|_| {
            PublishError::Validation("no se pudieron codificar los argumentos Candid".into())
        })?;
        self.ensure_local()?;
        // dialoguer::Password checks that stderr is a real terminal. Inherit it
        // for interactive updates so the official CLI owns the entire prompt.
        let output = execute(
            &mut self.call_command(method, args, query),
            !query && self.interactive,
        )?;
        let bytes = std::str::from_utf8(&output)
            .ok()
            .and_then(|s| hex::decode(s.trim()).ok())
            .ok_or_else(|| {
                PublishError::InvalidResponse(format!(
                    "{method}: respuesta hex inválida o no reconocida"
                ))
            })?;
        decode_response(&bytes)
    }
}

pub(super) fn decode_response<T: CandidType + DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, PublishError> {
    let result: Result<T, RegistryError> = candid::decode_one(bytes).map_err(|_| {
        PublishError::InvalidResponse("respuesta Candid inválida o no reconocida".into())
    })?;
    result.map_err(PublishError::Registry)
}

fn execute(command: &mut Command, interactive: bool) -> Result<Vec<u8>, PublishError> {
    if interactive {
        command.stderr(Stdio::inherit());
    }
    let output = command
        .output()
        .map_err(|_| PublishError::Cli(CliFailure::Process))?;
    if !output.status.success() {
        if interactive {
            // The CLI already displayed its precise error; no password or TTY
            // input is intercepted to recover the diagnostic.
            return Err(PublishError::Cli(CliFailure::InteractiveProcess));
        }
        // Do not log raw stderr, which belongs to the identity/CLI provider.
        let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
        let kind = if stderr.contains("invalid request expiry") || stderr.contains("ingress_expiry")
        {
            CliFailure::InvalidExpiry
        } else if [
            "password", "identity", "keyring", "decrypt", "terminal", "tty",
        ]
        .iter()
        .any(|s| stderr.contains(s))
        {
            CliFailure::IdentityUnavailable
        } else if [
            "connection",
            "connect",
            "timeout",
            "timed out",
            "transport",
            "dns",
            "tcp",
            "http",
            "network",
        ]
        .iter()
        .any(|s| stderr.contains(s))
        {
            CliFailure::Connection
        } else {
            CliFailure::Process
        };
        return Err(PublishError::Cli(kind));
    }
    Ok(output.stdout)
}

impl Registry for IcpCli {
    fn get_mission(&mut self, id: &str) -> Result<Option<Mission>, PublishError> {
        self.call("get_mission", candid::encode_args((id,)), true)
    }
    fn get_attempt(&mut self, id: &str) -> Result<Option<RemoteAttempt>, PublishError> {
        self.call("get_attempt", candid::encode_args((id,)), true)
    }
    fn list_attempts(
        &mut self,
        mission: &str,
        cursor: Option<String>,
    ) -> Result<Vec<RemoteAttempt>, PublishError> {
        self.call(
            "list_attempts",
            candid::encode_args((mission, cursor, 100_u16)),
            true,
        )
    }
    fn register_attempt(&mut self, input: &AttemptInput) -> Result<RemoteAttempt, PublishError> {
        self.call("register_attempt", candid::encode_args((input,)), false)
    }
}

#[cfg(test)]
mod tests;
