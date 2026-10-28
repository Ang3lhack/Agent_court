use super::*;
use crate::evidence::{self, Verification};

const GOOD: &str = include_str!("../../tests/fixtures/counter-pass.rs");
const BAD: &str = include_str!("../../tests/fixtures/counter-fail.rs");

fn workspace() -> Workspace {
    Workspace(crate::temporary_workspace().unwrap())
}
fn candidate(root: &Workspace, bytes: &[u8]) -> PathBuf {
    let path = root.0.join("candidate.rs");
    fs::write(&path, bytes).unwrap();
    path
}
fn no_docker() -> Docker {
    Docker {
        executable: "/no-such-agent-court-docker".into(),
        ..Docker::default()
    }
}

#[test]
fn missing_file_is_an_error_without_fabricated_artifact_or_package() {
    let root = workspace();
    assert!(
        evaluate_with(&root.0.join("absent.rs"), false, &no_docker())
            .err()
            .unwrap()
            .contains("no se puede leer")
    );
    assert!(fs::read_dir(&root.0).unwrap().next().is_none());
}

#[test]
fn source_limit_and_non_regular_inputs_are_rejected_before_execution() {
    let root = workspace();
    let path = candidate(&root, &vec![b'x'; SOURCE_LIMIT as usize + 1]);
    assert!(evaluate_with(&path, false, &no_docker())
        .err()
        .unwrap()
        .contains("256 KiB"));
    assert!(evaluate_with(&root.0, false, &no_docker()).is_err());
    #[cfg(unix)]
    {
        let link = root.0.join("link.rs");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(evaluate_with(&link, false, &no_docker()).is_err());
    }
}

#[test]
fn absent_docker_is_error_with_exact_external_artifact_and_verifiable_evidence() {
    let root = workspace();
    let bytes = format!("{GOOD}\n// exact trailing bytes\r\n").into_bytes();
    let evaluation = evaluate_with(&candidate(&root, &bytes), false, &no_docker()).unwrap();
    assert_eq!(evaluation.report.verdict, Status::Error);
    assert_eq!(evaluation.report.artifact_hash, sha256(&bytes));
    assert_ne!(
        evaluation.report.artifact_hash,
        sha256(crate::TEST_SOURCE.as_bytes())
    );
    let store = root.0.join("store");
    let record = evidence::record_attempt(&store, &evaluation).unwrap();
    assert_eq!(
        fs::read(
            store
                .join("attempts")
                .join(&record.attempt_id)
                .join("artifact.rs")
        )
        .unwrap(),
        bytes
    );
    assert!(matches!(
        evidence::verify_attempt(&store, &record.attempt_id),
        Ok(Verification::Verified(_))
    ));
}

#[test]
fn explicit_no_run_skips_docker_and_preserves_existing_chain_and_package() {
    let root = workspace();
    let path = candidate(&root, GOOD.as_bytes());
    let store = root.0.join("store");
    let first = evaluate_with(&path, true, &no_docker()).unwrap();
    assert_eq!(first.report.verdict, Status::NotRun);
    let first = evidence::record_attempt(&store, &first).unwrap();
    let log = fs::read(store.join("evidence.jsonl")).unwrap();
    let package = store.join("attempts").join(&first.attempt_id);
    let before: Vec<_> = fs::read_dir(&package)
        .unwrap()
        .map(|e| {
            let path = e.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    let second = evaluate_with(&path, false, &no_docker()).unwrap();
    let second = evidence::record_attempt(&store, &second).unwrap();
    assert_eq!(second.previous_attempt_id.as_ref(), Some(&first.attempt_id));
    assert!(fs::read(store.join("evidence.jsonl"))
        .unwrap()
        .starts_with(&log));
    for (path, bytes) in before {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    for id in [&first.attempt_id, &second.attempt_id] {
        assert!(matches!(
            evidence::verify_attempt(&store, id),
            Ok(Verification::Verified(_))
        ));
    }
}

#[test]
fn prepared_mount_contains_only_exact_candidate_and_immutable_evaluator_sources() {
    let root = Workspace::prepare(GOOD.as_bytes()).unwrap();
    let input = root.0.join("input");
    assert_eq!(fs::read_dir(&input).unwrap().count(), 3);
    assert_eq!(
        fs::read(input.join("candidate.rs")).unwrap(),
        GOOD.as_bytes()
    );
    assert_eq!(fs::read(input.join("suite.rs")).unwrap(), SUITE.as_bytes());
    assert_eq!(
        fs::read(input.join("driver.sh")).unwrap(),
        DRIVER.as_bytes()
    );
    assert!(!SUITE.contains("struct Counter"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for name in ["candidate.rs", "suite.rs", "driver.sh"] {
            assert_eq!(
                fs::metadata(input.join(name)).unwrap().permissions().mode() & 0o222,
                0
            );
        }
    }
}

#[test]
fn docker_arguments_apply_limits_and_mount_only_the_read_only_copy() {
    let docker = Docker::default();
    let args = docker
        .create_args(
            Path::new("/tmp/isolated/input"),
            "own-container",
            "sha256:image",
        )
        .unwrap();
    for flag in [
        "--network=none",
        "--cap-drop=ALL",
        "--security-opt=no-new-privileges=true",
        "--user=65534:65534",
        "--read-only",
        "--cpus=1",
        "--memory=512m",
        "--memory-swap=512m",
        "--pids-limit=128",
        "--pull=never",
        "--log-driver=none",
    ] {
        assert!(args.iter().any(|arg| arg == flag), "missing {flag}");
    }
    assert_eq!(
        args.iter().filter(|arg| arg.as_str() == "--mount").count(),
        1
    );
    assert!(args.contains(&"type=bind,src=/tmp/isolated/input,dst=/input,readonly".into()));
    assert!(!args.iter().any(|arg| arg.contains("docker.sock")
        || arg.contains("/home/")
        || arg == "--privileged"
        || arg == "--use-api-socket"));
    assert!(docker
        .create_args(Path::new("/tmp/unsafe,other"), "own", "id")
        .is_err());
}

#[test]
fn verdict_requires_daemon_state_and_never_trusts_candidate_stdout() {
    let mut output = Output {
        code: Some(0),
        stop: Stop::Completed,
        stdout: b"ALL TESTS PASSED".to_vec(),
        stderr: vec![],
    };
    let mut state = ContainerState {
        status: "exited".into(),
        exit_code: 0,
        oom_killed: false,
        error: String::new(),
    };
    for (code, expected) in [
        (0, Status::Pass),
        (10, Status::Fail),
        (11, Status::Fail),
        (12, Status::Fail),
    ] {
        output.code = Some(code);
        state.exit_code = code;
        assert_eq!(classify(&output, &state).unwrap().0, expected);
    }
    output.code = Some(125);
    state.exit_code = 0;
    assert!(classify(&output, &state).is_err());
    output.code = Some(20);
    state.exit_code = 20;
    assert!(classify(&output, &state).is_err());
    output.stop = Stop::Timeout;
    state.status = "running".into();
    assert_eq!(classify(&output, &state).unwrap().0, Status::Fail);
    state.status = "created".into();
    assert!(classify(&output, &state).is_err());
    state.oom_killed = true;
    assert_eq!(classify(&output, &state).unwrap().0, Status::Fail);
}

#[test]
#[cfg(unix)]
fn bounded_control_process_enforces_deadline_and_combined_output_limit() {
    let start = std::time::Instant::now();
    let result = process::run(
        Command::new("sh").args(["-c", "exec sleep 5"]),
        Duration::from_millis(100),
    )
    .unwrap();
    assert_eq!(result.stop, Stop::Timeout);
    assert!(start.elapsed() < Duration::from_secs(2));
    let result = process::run(
        Command::new("sh").args([
            "-c",
            "while :; do printf 'bounded output\n'; printf 'bounded error\n' >&2; done",
        ]),
        Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(result.stop, Stop::OutputLimit);
    assert_eq!(
        result.stdout.len() + result.stderr.len(),
        process::OUTPUT_LIMIT
    );
}

#[test]
#[cfg(unix)]
fn mocked_docker_lifecycle_reconciles_state_and_removes_its_own_container() {
    let root = workspace();
    let path = candidate(&root, GOOD.as_bytes());
    let docker = Docker {
        executable: Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/docker.sh"),
        cwd: root.0.clone(),
        run_timeout: RUN_TIMEOUT,
    };
    fs::write(root.0.join("output"), "untrusted candidate output\n").unwrap();
    for (code, expected) in [
        (0, Status::Pass),
        (10, Status::Fail),
        (11, Status::Fail),
        (20, Status::Error),
    ] {
        fs::write(root.0.join("exit-code"), format!("{code}\n")).unwrap();
        fs::write(root.0.join("state"), format!("{{\"Status\":\"exited\",\"ExitCode\":{code},\"OOMKilled\":false,\"Error\":\"\"}}\n")).unwrap();
        assert_eq!(
            evaluate_with(&path, false, &docker).unwrap().report.verdict,
            expected
        );
    }
    let removed = fs::read_to_string(root.0.join("removed")).unwrap();
    assert_eq!(removed.lines().filter(|line| *line == "--force").count(), 4);
    assert!(fs::read_to_string(root.0.join("create-args"))
        .unwrap()
        .contains("--network=none"));
}

#[test]
#[cfg(unix)]
fn invalid_state_connection_failure_and_failed_cleanup_cannot_be_pass() {
    let root = workspace();
    let path = candidate(&root, GOOD.as_bytes());
    let docker = Docker {
        executable: Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/docker.sh"),
        cwd: root.0.clone(),
        run_timeout: RUN_TIMEOUT,
    };
    fs::write(root.0.join("output"), "fake successful test text\n").unwrap();
    fs::write(root.0.join("exit-code"), "0\n").unwrap();
    fs::write(root.0.join("state"), "not JSON\n").unwrap();
    assert_eq!(
        evaluate_with(&path, false, &docker).unwrap().report.verdict,
        Status::Error
    );
    assert!(root.0.join("removed").exists());
    fs::write(
        root.0.join("state"),
        "{\"Status\":\"exited\",\"ExitCode\":0,\"OOMKilled\":false,\"Error\":\"\"}\n",
    )
    .unwrap();
    fs::write(root.0.join("exit-code"), "125\n").unwrap();
    assert_eq!(
        evaluate_with(&path, false, &docker).unwrap().report.verdict,
        Status::Error
    );
    fs::write(root.0.join("exit-code"), "0\n").unwrap();
    fs::write(root.0.join("rm-code"), "1\n").unwrap();
    assert_eq!(
        evaluate_with(&path, false, &docker).unwrap().report.verdict,
        Status::Error
    );
    fs::write(root.0.join("rm-code"), "0\n").unwrap();
    fs::write(root.0.join("create-id"), "invalid response\n").unwrap();
    let error = evaluate_with(&path, false, &docker).unwrap();
    assert_eq!(error.report.verdict, Status::Error);
    assert!(error.report.checks[0]
        .detail
        .contains("container ID no reconocido"));
    assert!(!error.report.checks[0]
        .detail
        .contains("no se confirmó la limpieza"));
    fs::write(root.0.join("create-code"), "125\n").unwrap();
    assert_eq!(
        evaluate_with(&path, false, &docker).unwrap().report.verdict,
        Status::Error
    );
}

/// Opt-in: real Docker only, no ICP calls, all evidence in a test-owned directory.
#[test]
#[ignore = "requires the Docker daemon and pre-pulled official Rust image"]
fn docker_counter_files() {
    let root = workspace();
    let docker = Docker::default();
    let mut cases = vec![
        ("correct", GOOD.to_string(), Status::Pass),
        ("logic", BAD.to_string(), Status::Fail),
        ("compile", "this is not Rust".into(), Status::Fail),
        ("wrong-contract", "pub struct Counter;".into(), Status::Fail),
        (
            "early-exit",
            GOOD.replace("self.0 += 1;", "std::process::exit(0);"),
            Status::Fail,
        ),
        (
            "timeout",
            GOOD.replace(
                "self.0 += 1;",
                "loop { std::thread::sleep(std::time::Duration::from_millis(10)); }",
            ),
            Status::Fail,
        ),
        (
            "output-limit",
            GOOD.replace("self.0 += 1;", "loop { println!(\"untrusted output\"); }"),
            Status::Fail,
        ),
        (
            "candidate-tests-excluded",
            format!("{GOOD}\n#[test] fn fake_test() {{ panic!(\"must not run\"); }}"),
            Status::Pass,
        ),
    ];
    let security = GOOD.replace("#[derive(Default)]", "").replace(
        "pub struct Counter(u64);",
        r#"pub struct Counter(u64);
impl Default for Counter {
    fn default() -> Self {
        assert!(!std::path::Path::new("/var/run/docker.sock").exists());
        assert!(!std::path::Path::new("/home/lomeg/projects/Agent_court").exists());
        assert!(std::fs::OpenOptions::new().write(true).open("/input/suite.rs").is_err());
        assert!(std::fs::File::create("/etc/agent-court-probe").is_err());
        assert_eq!(std::env::var("HOME").unwrap(), "/work");
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        assert!(status.contains("CapEff:\t0000000000000000"));
        assert!(status.contains("NoNewPrivs:\t1"));
        assert!(!std::path::Path::new("/sys/class/net/eth0").exists());
        Self(0)
    }
}"#,
    );
    cases.push(("isolation", security, Status::Pass));
    let store = root.0.join("evidence");
    let mut previous = None;
    for (name, source, expected) in cases {
        let evaluation =
            evaluate_with(&candidate(&root, source.as_bytes()), false, &docker).unwrap();
        assert_eq!(
            evaluation.report.verdict, expected,
            "{name}: {:?}",
            evaluation.report.checks
        );
        assert_eq!(evaluation.report.artifact_hash, sha256(source.as_bytes()));
        let record = evidence::record_attempt(&store, &evaluation).unwrap();
        assert_eq!(record.previous_attempt_id, previous);
        assert!(matches!(
            evidence::verify_attempt(&store, &record.attempt_id),
            Ok(Verification::Verified(_))
        ));
        previous = Some(record.attempt_id);
        println!("Docker real {name}: {expected:?}, artefacto exacto y paquete verificado");
    }
}
