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
    ] {
        assert_eq!(workspace.run(&args).status.code(), Some(64));
        assert!(!workspace.0.join(".agent-court").exists());
    }
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
            &args[..6],
            &["exec", "--json", "--sandbox", sandbox, "--cd", "."]
        );
        assert_eq!(args[6], "--output-last-message");
        assert_eq!(args[8], "test prompt with spaces");
        assert!(!Path::new(args[7]).exists());
    }
    assert!(!workspace.0.join(".agent-court").exists());
}
