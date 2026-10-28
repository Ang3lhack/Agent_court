use super::*;

struct TestFiles(PathBuf);
impl TestFiles {
    fn new() -> Self {
        Self(crate::temporary_workspace().unwrap())
    }
    #[cfg(unix)]
    fn script(&self, source: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = self.0.join("controlled-codex");
        fs::write(&path, format!("#!/bin/sh\n{source}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
    fn request(&self) -> AgentRequest {
        AgentRequest {
            role: AgentRole::Builder,
            prompt: "controlled prompt".into(),
            working_dir: self.0.clone(),
        }
    }
}
impl Drop for TestFiles {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn missing_codex_and_invalid_requests_fail_without_fabricated_results() {
    let files = TestFiles::new();
    let error = run_codex(
        files.request(),
        files.0.join("absent-codex"),
        Duration::from_secs(1),
    )
    .unwrap_err();
    assert!(error.contains("cannot run codex exec"));
    let mut request = files.request();
    request.prompt.clear();
    assert!(CodexRunner
        .run(request)
        .unwrap_err()
        .contains("prompt must contain"));
    let mut request = files.request();
    request.working_dir = files.0.join("absent");
    assert!(CodexRunner
        .run(request)
        .unwrap_err()
        .contains("does not exist"));
}

#[test]
#[cfg(unix)]
fn controlled_codex_timeout_and_output_are_bounded_and_descendants_are_stopped() {
    let files = TestFiles::new();
    let executable = files.script("sleep 20 &\nprintf '%s' \"$!\" > descendant\nwait");
    let start = std::time::Instant::now();
    let error = run_codex(files.request(), executable, Duration::from_millis(100)).unwrap_err();
    assert!(error.contains("Timeout"), "{error}");
    assert!(start.elapsed() < Duration::from_secs(3));
    let pid = fs::read_to_string(files.0.join("descendant")).unwrap();
    // A killed process may briefly remain as a zombie until init reaps it.
    if let Ok(status) = fs::read_to_string(format!("/proc/{pid}/status")) {
        assert!(
            status
                .lines()
                .any(|line| line.starts_with("State:") && line.contains('Z')),
            "{status}"
        );
    }
    let executable = files.script(
        "while :; do printf 'controlled stdout spam'; printf 'controlled stderr spam' >&2; done",
    );
    let error = run_codex(files.request(), executable, Duration::from_secs(3)).unwrap_err();
    assert!(error.contains("OutputLimit"), "{error}");
    assert!(error.len() <= AGENT_OUTPUT_LIMIT + 200);
}

#[test]
#[cfg(unix)]
fn nonzero_exit_and_oversized_last_message_never_become_agent_success() {
    let files = TestFiles::new();
    let executable = files.script("printf 'controlled auth failure' >&2; exit 7");
    let result = run_codex(files.request(), executable, Duration::from_secs(2)).unwrap();
    assert!(!result.success);
    assert_eq!(result.exit_code, Some(7));
    assert!(result.stderr.contains("controlled auth failure"));
    let executable = files.script("while [ \"$1\" != '--output-last-message' ]; do shift; done\nshift\nhead -c 17000 /dev/zero > \"$1\"");
    assert!(
        run_codex(files.request(), executable, Duration::from_secs(2))
            .unwrap_err()
            .contains("16 KiB")
    );
}
