//! External Rust is compiled and executed only by the Docker daemon.
use crate::{sha256, Check, Evaluation, Status};
use serde::Deserialize;
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

mod process;
use process::{Output, Stop};

const IMAGE: &str = "rust:1.89.0-slim-bookworm";
const SOURCE_LIMIT: u64 = 256 * 1024;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);
const RUN_TIMEOUT: Duration = Duration::from_secs(30);
const SUITE: &str = include_str!("candidate/suite.rs");
const DRIVER: &str = include_str!("candidate/driver.sh");

struct Workspace(PathBuf);
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl Workspace {
    fn prepare(artifact: &[u8]) -> Result<Self, String> {
        let root = Self(crate::temporary_workspace()?);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root.0, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        let input = root.0.join("input");
        fs::create_dir(&input).map_err(|e| e.to_string())?;
        for (name, bytes) in [
            ("candidate.rs", artifact),
            ("suite.rs", SUITE.as_bytes()),
            ("driver.sh", DRIVER.as_bytes()),
        ] {
            let path = input.join(name);
            fs::write(&path, bytes).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o444))
                    .map_err(|e| e.to_string())?;
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&input, fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
        }
        Ok(root)
    }
}

struct Docker {
    executable: PathBuf,
    cwd: PathBuf,
    run_timeout: Duration,
}

impl Default for Docker {
    fn default() -> Self {
        Self {
            executable: "docker".into(),
            cwd: std::env::temp_dir(),
            run_timeout: RUN_TIMEOUT,
        }
    }
}

impl Docker {
    fn run(&self, args: &[&str], timeout: Duration) -> Result<Output, String> {
        process::run(
            Command::new(&self.executable)
                .current_dir(&self.cwd)
                .args(args),
            timeout,
        )
    }
    fn control(&self, args: &[&str]) -> Result<Output, String> {
        let output = self.run(args, CONTROL_TIMEOUT)?;
        if !output.success() {
            return Err(format!("Docker {}: {}", args[0], output.diagnostic()));
        }
        Ok(output)
    }
    fn preflight(&self) -> Result<String, String> {
        let info = self.control(&["info", "--format", "{{.ServerVersion}}"])?;
        if info.stdout.iter().all(u8::is_ascii_whitespace) {
            return Err("daemon Docker sin versión reconocible".into());
        }
        let image = self
            .control(&["image", "inspect", "--format", "{{.Id}}", IMAGE])
            .map_err(|error| {
                format!("imagen {IMAGE} no disponible; prepárala con docker pull {IMAGE}: {error}")
            })?;
        let image = String::from_utf8(image.stdout).map_err(|_| "ID de imagen inválido")?;
        let image = image.trim();
        if !image
            .strip_prefix("sha256:")
            .is_some_and(valid_container_id)
        {
            return Err("ID de imagen Docker inválido".into());
        }
        Ok(image.into())
    }
    fn create_args(&self, input: &Path, name: &str, image: &str) -> Result<Vec<String>, String> {
        let input = input
            .to_str()
            .filter(|path| !path.contains([',', '\n', '\r']))
            .ok_or("ruta temporal incompatible con un bind mount Docker")?;
        Ok([
            "create",
            "--name",
            name,
            "--label",
            &format!("agent-court.owner={name}"),
            "--pull=never",
            "--network=none",
            "--cap-drop=ALL",
            "--security-opt=no-new-privileges=true",
            "--user=65534:65534",
            "--read-only",
            "--init",
            "--cpus=1",
            "--memory=512m",
            "--memory-swap=512m",
            "--pids-limit=128",
            "--ulimit=nofile=128:128",
            "--ulimit=core=0:0",
            "--ulimit=fsize=67108864:67108864",
            "--log-driver=none",
            "--shm-size=1m",
            "--ipc=private",
            "--workdir=/work",
            "--env=HOME=/work",
            "--env=TMPDIR=/work",
            "--tmpfs=/work:rw,exec,nosuid,nodev,size=128m,mode=1777",
            "--tmpfs=/tmp:rw,noexec,nosuid,nodev,size=16m,mode=1777",
            "--mount",
            &format!("type=bind,src={input},dst=/input,readonly"),
            "--entrypoint=/bin/sh",
            image,
            "/input/driver.sh",
        ]
        .into_iter()
        .map(String::from)
        .collect())
    }
    fn execute(&self, artifact: &[u8], image: &str) -> Result<(Status, String), String> {
        let workspace = Workspace::prepare(artifact)?;
        let name = workspace
            .0
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("nombre temporal inválido")?;
        let args = self.create_args(&workspace.0.join("input"), name, image)?;
        let args: Vec<_> = args.iter().map(String::as_str).collect();
        let created = self.control(&args).and_then(|output| {
            let id = String::from_utf8(output.stdout).map_err(|_| "container ID no reconocido")?;
            if !valid_container_id(id.trim()) {
                return Err("container ID no reconocido".into());
            }
            Ok(id.trim().to_owned())
        });
        let id = match created {
            Ok(id) => id,
            Err(error) => {
                if let Err(cleanup) = self.reconcile_creation(name) {
                    return Err(format!(
                        "{error}; no se confirmó la limpieza de {name}: {cleanup}"
                    ));
                }
                return Err(error);
            }
        };
        let mut container = Container {
            docker: self,
            id: id.clone(),
            removed: false,
        };
        let result = (|| {
            let output = self.run(&["start", "--attach", &id], self.run_timeout)?;
            let state = self.control(&["inspect", "--format", "{{json .State}}", &id])?;
            let state: ContainerState =
                serde_json::from_slice(&state.stdout).map_err(|_| "estado Docker inválido")?;
            classify(&output, &state)
        })();
        container.remove()?; // Cleanup failure is infrastructure ERROR, never PASS.
        result
    }

    fn reconcile_creation(&self, name: &str) -> Result<(), String> {
        // A lost/malformed create response is never permission to delete an
        // unrelated container. Require our unique owner label and immutable ID.
        let found = self.control(&[
            "inspect",
            "--format",
            "{{.Id}} {{index .Config.Labels \"agent-court.owner\"}}",
            name,
        ])?;
        let text = String::from_utf8_lossy(&found.stdout);
        let fields: Vec<_> = text.split_whitespace().collect();
        if fields.len() != 2 || !valid_container_id(fields[0]) || fields[1] != name {
            return Err("no se reconoció un contenedor propio; no se eliminó ningún otro".into());
        }
        Container {
            docker: self,
            id: fields[0].into(),
            removed: false,
        }
        .remove()
    }
}

fn valid_container_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

struct Container<'a> {
    docker: &'a Docker,
    id: String,
    removed: bool,
}
impl Container<'_> {
    fn remove(&mut self) -> Result<(), String> {
        self.docker
            .control(&["rm", "--force", &self.id])
            .map_err(|error| {
                format!(
                    "no se pudo eliminar el contenedor propio {}: {error}",
                    self.id
                )
            })?;
        self.removed = true;
        Ok(())
    }
}
impl Drop for Container<'_> {
    fn drop(&mut self) {
        if !self.removed {
            let _ = self.remove();
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ContainerState {
    status: String,
    exit_code: i32,
    #[serde(rename = "OOMKilled")]
    oom_killed: bool,
    error: String,
}

fn classify(output: &Output, state: &ContainerState) -> Result<(Status, String), String> {
    if !state.error.is_empty() {
        return Err(format!("runtime Docker: {}", state.error));
    }
    let detail = output.diagnostic();
    if state.oom_killed {
        return Ok((
            Status::Fail,
            format!("límite de memoria del candidato; {detail}"),
        ));
    }
    if output.stop != Stop::Completed {
        if state.status == "running" || state.status == "exited" {
            return Ok((
                Status::Fail,
                format!("límite de tiempo o salida del candidato; {detail}"),
            ));
        }
        return Err(format!(
            "Docker no llegó a ejecutar el contenedor; {detail}"
        ));
    }
    if state.status != "exited" || output.code != Some(state.exit_code) {
        return Err(format!("estado/salida Docker incoherentes; {detail}"));
    }
    match state.exit_code {
        0 => Ok((Status::Pass, format!("batería independiente completada: inicio, incrementos, consulta y reinicio; {detail}"))),
        10 => Ok((Status::Fail, format!("compilación del candidato o contrato público inválido; {detail}"))),
        11 => Ok((Status::Fail, format!("pruebas del candidato fallidas o terminación prematura; {detail}"))),
        12 => Ok((Status::Fail, format!("límite de tiempo/recursos del candidato; {detail}"))),
        _ => Err(format!("fallo de infraestructura/toolchain (driver); {detail}")),
    }
}

fn read_candidate(path: &Path) -> Result<Vec<u8>, String> {
    if !fs::symlink_metadata(path)
        .map_err(|e| format!("no se puede leer {}: {e}", path.display()))?
        .is_file()
    {
        return Err(
            "el candidato debe ser un archivo regular, no un directorio ni un enlace".into(),
        );
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(SOURCE_LIMIT + 1).read_to_end(&mut bytes))
        .map_err(|e| format!("no se puede leer el candidato: {e}"))?;
    if bytes.len() as u64 > SOURCE_LIMIT {
        return Err("candidate.rs supera el límite de 256 KiB".into());
    }
    Ok(bytes)
}

pub fn evaluate_file(path: &Path, no_run: bool) -> Result<Evaluation, String> {
    evaluate_with(path, no_run, &Docker::default())
}

fn evaluate_with(path: &Path, no_run: bool, docker: &Docker) -> Result<Evaluation, String> {
    let artifact = read_candidate(path)?;
    let mut image = format!("requested={IMAGE}; not executed");
    let (verdict, detail) = if no_run {
        (
            Status::NotRun,
            "ejecución omitida explícitamente mediante --no-run; Docker no fue invocado".into(),
        )
    } else {
        match docker.preflight().and_then(|id| {
            image = format!("tag={IMAGE}; image_id={id}");
            docker.execute(&artifact, &id)
        }) {
            Ok(result) => result,
            Err(error) => (Status::Error, error),
        }
    };
    let isolation = format!("Docker Linux; {image}; read-only input/root; uid=65534; network=none; cap-drop=ALL; no-new-privileges; cpu=1; memory/swap=512MiB; pids=128; work=128MiB; tmp=16MiB; wall={}s; output={}B; compile=10s each; tests=5s; suite_sha256={}; driver_sha256={}", docker.run_timeout.as_secs(), process::OUTPUT_LIMIT, sha256(SUITE.as_bytes()), sha256(DRIVER.as_bytes()));
    Ok(Evaluation::from_artifact(
        verdict,
        vec![Check {
            name: "counter_external".into(),
            status: verdict,
            detail,
        }],
        artifact,
        isolation,
    ))
}

#[cfg(test)]
mod tests;
