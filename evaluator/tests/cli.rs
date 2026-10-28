use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "agent-court-cli-test-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_agent-court-evaluator"))
            .current_dir(&self.0)
            .args(args)
            .output()
            .unwrap()
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn snapshot(path: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut result = Vec::new();
    if !path.exists() {
        return result;
    }
    for entry in fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            result.extend(snapshot(&entry.path()));
        } else {
            result.push((entry.path(), fs::read(entry.path()).unwrap()));
        }
    }
    result.sort();
    result
}

#[test]
fn executable_generates_and_verifies_an_independent_store_without_touching_default_history() {
    let workspace = Workspace::new();
    let original = workspace.0.join(".agent-court/evidence.jsonl");
    fs::create_dir_all(original.parent().unwrap()).unwrap();
    // Deliberately invalid default store: the selected store must be independent.
    let historical = b"historical-1\nhistorical-2\nhistorical-3\nhistorical-4\n";
    fs::write(&original, historical).unwrap();
    let generated = workspace.run(&["--evidence-dir", ".agent-court/demo-stage4"]);
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    let stdout = String::from_utf8(generated.stdout).unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&stdout[stdout.find('{').unwrap()..]).unwrap();
    let id = json["evidence"]["attempt_id"].as_str().unwrap();
    assert!(json["evidence"]["previous_attempt_id"].is_null());
    assert_eq!(json["report"]["verdict"], "PASS");
    let store = workspace.0.join(".agent-court/demo-stage4");
    let before = snapshot(&store);
    let verified = workspace.run(&["--evidence-dir", ".agent-court/demo-stage4", "verify", id]);
    assert!(verified.status.success());
    assert!(String::from_utf8_lossy(&verified.stdout).contains("VERIFICADO"));
    assert_eq!(before, snapshot(&store));
    let artifact = store.join("attempts").join(id).join("artifact.rs");
    fs::write(&artifact, "modified").unwrap();
    let tampered = snapshot(&store);
    let rejected = workspace.run(&["--evidence-dir", ".agent-court/demo-stage4", "publish", id]);
    assert_eq!(rejected.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("SHA-256 mismatch"));
    assert_eq!(tampered, snapshot(&store));
    assert_eq!(fs::read(original).unwrap(), historical);
}

#[test]
fn usage_errors_do_not_create_evidence() {
    let workspace = Workspace::new();
    for args in [
        vec!["publish"],
        vec!["verify"],
        vec!["--evidence-dir"],
        vec!["publish", "a", "--network", "ic"],
        vec!["agent"],
        vec!["evaluate-file"],
        vec!["evaluate-file", "candidate.rs", "--unknown"],
        vec!["orchestrate", "counter-v2"],
        vec!["orchestrate", "--identity", "controller"],
        vec!["orchestrate", "--publish", "--identity", "../bad"],
        vec!["orchestrate", "--no-run"],
    ] {
        assert_eq!(workspace.run(&args).status.code(), Some(64));
        assert!(!workspace.0.join(".agent-court").exists());
    }
}

#[test]
fn orchestrate_without_codex_stops_before_evaluation_or_evidence_creation() {
    let workspace = Workspace::new();
    let output = Command::new(env!("CARGO_BIN_EXE_agent-court-evaluator"))
        .current_dir(&workspace.0)
        .env("PATH", &workspace.0)
        .args(["orchestrate", "counter-v1", "--evidence-dir", "store"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("BUILDER START"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!stdout.contains("EVALUATOR 1/2"));
    assert!(!stdout.contains("VERDICT PASS"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot run codex exec"));
    assert!(!workspace.0.join("store").exists());
    assert!(!workspace.0.join(".agent-court").exists());
}

#[test]
fn demo_cli_rejects_publication_and_invalid_seeds_before_creating_evidence() {
    let workspace = Workspace::new();
    for (args, code) in [
        (
            vec![
                "--evidence-dir",
                "store",
                "orchestrate",
                "counter-v1",
                "--demo-seed",
                "absent.rs",
                "--publish",
            ],
            64,
        ),
        (
            vec!["orchestrate", "--publish", "--demo-seed", "absent.rs"],
            64,
        ),
        (vec!["orchestrate", "--demo-seed"], 64),
        (
            vec!["orchestrate", "--demo-seed", "a.rs", "--demo-seed", "b.rs"],
            64,
        ),
        (
            vec![
                "--evidence-dir",
                "store",
                "orchestrate",
                "--demo-seed",
                "absent.rs",
            ],
            2,
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_agent-court-evaluator"))
            .current_dir(&workspace.0)
            .env("PATH", &workspace.0) // Never fall back to a real agent or ICP.
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(code),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if args.contains(&"--publish") {
            assert!(String::from_utf8_lossy(&output.stderr)
                .contains("--demo-seed no permite --publish"));
        }
        assert!(!String::from_utf8_lossy(&output.stdout).contains("START"));
        assert!(!workspace.0.join("store").exists());
        assert!(!workspace.0.join(".agent-court").exists());
    }
}

#[test]
fn demo_cli_without_docker_records_real_infrastructure_error_and_skips_all_agents() {
    let workspace = Workspace::new();
    let seed = workspace.0.join("provided.rs");
    let source = include_bytes!("fixtures/counter-fail.rs");
    fs::write(&seed, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_agent-court-evaluator"))
        .current_dir(&workspace.0)
        .env("PATH", &workspace.0) // No Docker, Codex or ICP.
        .args([
            "orchestrate",
            "counter-v1",
            "--demo-seed",
            "provided.rs",
            "--evidence-dir",
            "store",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("DEMO CONTROLADA — candidato inicial proporcionado"));
    assert!(stdout.contains("BUILDER OMITIDO"));
    assert!(!stdout.contains("BUILDER START"));
    assert!(!stdout.contains("REVIEWER START"));
    assert!(!stdout.contains("REPAIR START"));
    assert!(stdout.contains("VERDICT ERROR attempt_id="));
    assert!(stdout.contains("MISSION END verdict=ERROR evaluations=1 repaired=false"));
    assert_eq!(fs::read(&seed).unwrap(), source);
    let log = fs::read_to_string(workspace.0.join("store/evidence.jsonl")).unwrap();
    assert_eq!(log.lines().count(), 1);
    let record: serde_json::Value = serde_json::from_str(log.trim()).unwrap();
    assert_eq!(record["verdict"], "ERROR");
    let id = record["attempt_id"].as_str().unwrap();
    assert_eq!(
        fs::read(
            workspace
                .0
                .join("store/attempts")
                .join(id)
                .join("artifact.rs")
        )
        .unwrap(),
        source
    );
    assert!(workspace
        .run(&["--evidence-dir", "store", "verify", id])
        .status
        .success());
    assert!(!workspace.0.join(".agent-court").exists());
}

/// Production CLI/CodexRunner with a controlled executable fixture, REAL Docker.
/// Never invokes a model, publishes to ICP, or writes repository evidence.
#[test]
#[cfg(unix)]
#[ignore = "requires Docker and its Rust image; Codex is a simulated executable"]
fn docker_cli_demo_uses_fixture_codex_only_for_review_and_repair() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    for final_verdict in ["PASS", "FAIL"] {
        let workspace = Workspace::new();
        let seed = workspace.0.join("provided.rs");
        let original = include_bytes!("fixtures/counter-fail.rs");
        fs::write(&seed, original).unwrap();
        fs::set_permissions(&seed, fs::Permissions::from_mode(0o444)).unwrap();
        let repaired: &[u8] = if final_verdict == "PASS" {
            include_bytes!("fixtures/counter-pass.rs")
        } else {
            original
        };
        fs::write(workspace.0.join("repaired.rs"), repaired).unwrap();
        let fake = workspace.0.join("codex");
        fs::write(
            &fake,
            r#"#!/bin/sh
# Explicit test double. No real model is called.
sandbox=''
message=''
while [ "$#" -gt 0 ]; do
    case "$1" in
        --sandbox) shift; sandbox="$1" ;;
        --output-last-message) shift; message="$1" ;;
    esac
    shift
done
printf '%s\n' "$sandbox" >> "$AGENT_COURT_DEMO_TEST_DIR/roles"
/bin/cat > "$AGENT_COURT_DEMO_TEST_DIR/prompt-$sandbox"
case "$sandbox" in
    read-only) printf '%s' 'controlled fixture review of real Docker error' > "$message" ;;
    workspace-write)
        /bin/cp "$AGENT_COURT_DEMO_TEST_DIR/repaired.rs" candidate.rs || exit 7
        printf '%s' 'controlled fixture repair' > "$message"
        ;;
    *) exit 8 ;;
esac
printf '%s\n' '{"type":"controlled_fixture"}'
"#,
        )
        .unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o700)).unwrap();
        let docker = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|dir| dir.join("docker"))
            .find(|path| path.is_file())
            .expect("Docker CLI installed");
        symlink(
            fs::canonicalize(docker).unwrap(),
            workspace.0.join("docker"),
        )
        .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_agent-court-evaluator"))
            .current_dir(&workspace.0)
            .env("PATH", &workspace.0) // Only the fixture Codex and real Docker are available.
            .env("AGENT_COURT_DEMO_TEST_DIR", &workspace.0)
            .args([
                "--evidence-dir",
                "store",
                "orchestrate",
                "counter-v1",
                "--demo-seed",
                "provided.rs",
            ])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(if final_verdict == "PASS" { 0 } else { 1 }),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("DEMO CONTROLADA — candidato inicial proporcionado"));
        assert!(!stdout.contains("BUILDER START"));
        assert!(stdout.contains("VERDICT FAIL attempt_id="));
        assert_eq!(
            stdout.contains("VERDICT PASS attempt_id="),
            final_verdict == "PASS"
        );
        assert!(stdout.contains(&format!(
            "MISSION END verdict={final_verdict} evaluations=2 repaired=true"
        )));
        assert_eq!(
            fs::read_to_string(workspace.0.join("roles")).unwrap(),
            "read-only\nworkspace-write\n"
        );
        for sandbox in ["read-only", "workspace-write"] {
            let prompt = fs::read_to_string(workspace.0.join(format!("prompt-{sandbox}"))).unwrap();
            assert!(prompt.contains("DEMO CONTROLADA — candidato inicial proporcionado"));
            assert!(prompt.contains("assertion `left == right` failed"));
            if sandbox == "workspace-write" {
                assert!(prompt.contains("controlled fixture review of real Docker error"));
            }
        }
        assert_eq!(fs::read(&seed).unwrap(), original);
        assert_eq!(fs::metadata(&seed).unwrap().permissions().mode() & 0o222, 0);
        let log = fs::read_to_string(workspace.0.join("store/evidence.jsonl")).unwrap();
        let records: Vec<serde_json::Value> = log
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["verdict"], "FAIL");
        assert_eq!(records[1]["verdict"], final_verdict);
        assert_eq!(records[1]["previous_attempt_id"], records[0]["attempt_id"]);
        for (record, bytes) in records.iter().zip([original.as_slice(), repaired]) {
            let id = record["attempt_id"].as_str().unwrap();
            assert_eq!(
                fs::read(
                    workspace
                        .0
                        .join("store/attempts")
                        .join(id)
                        .join("artifact.rs")
                )
                .unwrap(),
                bytes
            );
            assert!(workspace
                .run(&["--evidence-dir", "store", "verify", id])
                .status
                .success());
        }
        assert!(!workspace.0.join(".agent-court").exists());
        println!("DEMO: Docker real, Codex simulado, FAIL -> {final_verdict}; original intacto, dos paquetes verificados");
    }
}

#[test]
fn orchestrate_rejects_tmpdir_inside_a_repository_before_starting_agents() {
    let workspace = Workspace::new();
    fs::create_dir(workspace.0.join(".git")).unwrap();
    fs::write(workspace.0.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_agent-court-evaluator"))
        .current_dir(&workspace.0)
        .env("PATH", &workspace.0)
        .env("TMPDIR", &workspace.0)
        .args(["--evidence-dir", "store", "orchestrate"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("fuera del repositorio y del almacén"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("BUILDER START"));
    assert!(!workspace.0.join("store").exists());
    assert_eq!(fs::read_dir(&workspace.0).unwrap().count(), 1);
}

#[test]
#[cfg(unix)]
fn orchestrate_with_fixture_codex_and_missing_docker_records_error_without_repair_or_icp() {
    use std::os::unix::fs::PermissionsExt;
    let workspace = Workspace::new();
    // Explicit executable test double: this script never invokes a model.
    let fixture = workspace.0.join("codex");
    fs::write(&fixture, format!("#!/bin/sh\n/bin/cat > /dev/null\n/bin/cat > candidate.rs <<'CANDIDATE'\n{}\nCANDIDATE\n", include_str!("fixtures/counter-pass.rs"))).unwrap();
    fs::set_permissions(&fixture, fs::Permissions::from_mode(0o700)).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_agent-court-evaluator"))
        .current_dir(&workspace.0)
        .env("PATH", &workspace.0) // No Docker, ICP, or real Codex on this PATH.
        .args(["--evidence-dir", "store", "orchestrate", "counter-v1"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("VERDICT ERROR attempt_id="),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("PUBLICATION LOCAL"));
    assert!(!stdout.contains("REVIEWER START"));
    assert!(!stdout.contains("REPAIR START"));
    assert!(stdout.contains("MISSION END verdict=ERROR evaluations=1 repaired=false"));
    let log = fs::read_to_string(workspace.0.join("store/evidence.jsonl")).unwrap();
    let lines: Vec<_> = log.lines().collect();
    assert_eq!(lines.len(), 1);
    let record: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(record["verdict"], "ERROR");
    assert!(workspace
        .run(&[
            "--evidence-dir",
            "store",
            "verify",
            record["attempt_id"].as_str().unwrap()
        ])
        .status
        .success());
    assert!(!workspace.0.join(".agent-court").exists());
}

#[test]
fn evaluate_file_cli_no_run_preserves_external_bytes_and_missing_input_creates_no_attempt() {
    let workspace = Workspace::new();
    let missing = workspace.run(&["--evidence-dir", "store", "evaluate-file", "missing.rs"]);
    assert_eq!(missing.status.code(), Some(2));
    assert!(!workspace.0.join("store").exists());
    let bytes = include_bytes!("fixtures/counter-pass.rs");
    fs::write(workspace.0.join("candidate.rs"), bytes).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_agent-court-evaluator"))
        .current_dir(&workspace.0)
        .env("PATH", &workspace.0)
        .args([
            "--evidence-dir",
            "store",
            "evaluate-file",
            "candidate.rs",
            "--no-run",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["report"]["verdict"], "NOT_RUN");
    let id = result["evidence"]["attempt_id"].as_str().unwrap();
    let store = workspace.0.join("store");
    assert_eq!(
        fs::read(store.join("attempts").join(id).join("artifact.rs")).unwrap(),
        bytes
    );
    assert!(workspace
        .run(&["--evidence-dir", "store", "verify", id])
        .status
        .success());
    assert!(!workspace.0.join(".agent-court").exists());
}

#[test]
#[cfg(unix)]
fn agent_adapter_runs_only_a_controlled_fake_codex_and_preserves_all_role_arguments() {
    let workspace = Workspace::new();
    let fake = workspace.0.join("codex");
    std::os::unix::fs::symlink(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex.sh"),
        fake,
    )
    .unwrap();
    for (role, sandbox) in [
        ("reviewer", "read-only"),
        ("builder", "workspace-write"),
        ("repair", "workspace-write"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_agent-court-evaluator"))
            .current_dir(&workspace.0)
            .env("PATH", &workspace.0)
            .args(["agent", role, ".", "test prompt with spaces"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["role"], role);
        assert_eq!(result["success"], true);
        assert_eq!(result["last_message"], "controlled test response");
        let args = fs::read_to_string(workspace.0.join("agent-args.txt")).unwrap();
        let args: Vec<_> = args.lines().collect();
        assert_eq!(
            &args[..5],
            &["exec", "--json", "--sandbox", sandbox, "--cd"]
        );
        assert_eq!(Path::new(args[5]), fs::canonicalize(&workspace.0).unwrap());
        assert_eq!(args[6], "--output-last-message");
        assert_eq!(args.last(), Some(&"-"));
        assert!(args.contains(&"--skip-git-repo-check"));
        assert!(args.contains(&"--ignore-user-config"));
        assert!(args.contains(&"approval_policy=\"never\""));
        assert!(args.contains(&"sandbox_workspace_write.writable_roots=[]"));
        assert!(args.contains(&"sandbox_workspace_write.exclude_slash_tmp=true"));
        assert!(args.contains(&"sandbox_workspace_write.exclude_tmpdir_env_var=true"));
        assert!(args.contains(&"features.shell_tool=false"));
        assert_eq!(
            fs::read_to_string(workspace.0.join("agent-prompt.txt")).unwrap(),
            "test prompt with spaces"
        );
        assert!(!Path::new(args[7]).exists());
    }
    assert!(!workspace.0.join(".agent-court").exists());
}
