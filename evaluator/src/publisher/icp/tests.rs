use super::*;

#[test]
fn command_uses_separate_binary_candid_arguments_and_explicit_local_target() {
    let cli = IcpCli::new("agent-court-local").unwrap();
    let encoded = candid::encode_args(("counter-v1",)).unwrap();
    let command = cli.call_command("get_mission", encoded.clone(), true);
    let args: Vec<_> = command.get_args().map(|s| s.to_str().unwrap()).collect();
    assert_eq!(
        &args[..9],
        &[
            "canister",
            "call",
            "backend",
            "get_mission",
            &hex::encode(encoded),
            "--args-format",
            "hex",
            "--output",
            "hex"
        ]
    );
    assert!(args.windows(2).any(|a| a == ["--environment", "local"]));
    assert!(args.windows(2).any(|a| a == ["--identity", "anonymous"]));
    assert!(args.contains(&"--query"));
    assert!(!args.contains(&"--identity-password-file"));
    let args: Vec<_> = cli
        .call_command("register_attempt", vec![], false)
        .get_args()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    assert!(args
        .windows(2)
        .any(|a| a == ["--identity", "agent-court-local"]));
    assert!(!args.contains(&"--query".into()));
    for variable in ["ICP_NETWORK", "ICP_ENVIRONMENT", "ICP_PROJECT_ROOT"] {
        assert!(command
            .get_envs()
            .any(|(name, value)| name == variable && value.is_none()));
    }
    assert!(IcpCli::new("admin --network ic").is_err());
}

#[test]
fn all_candid_err_variants_are_decoded_explicitly() {
    for error in [
        RegistryError::Unauthorized,
        RegistryError::NotFound,
        RegistryError::AlreadyExists,
        RegistryError::ChainConflict {
            expected_previous_attempt_id: None,
        },
        RegistryError::ChainConflict {
            expected_previous_attempt_id: Some("tip".into()),
        },
        RegistryError::InvalidInput("bad hash".into()),
        RegistryError::CapacityReached,
        RegistryError::StorageError("storage".into()),
    ] {
        let bytes = candid::encode_one(Result::<RemoteAttempt, _>::Err(error.clone())).unwrap();
        assert!(
            matches!(decode_response::<RemoteAttempt>(&bytes), Err(PublishError::Registry(actual)) if actual == error)
        );
    }
}

#[test]
fn malformed_or_unrecognized_candid_responses_fail_closed() {
    for bytes in [
        b"not Candid".to_vec(),
        candid::encode_one("Ok").unwrap(),
        candid::encode_args(()).unwrap(),
    ] {
        assert!(matches!(
            decode_response::<RemoteAttempt>(&bytes),
            Err(PublishError::InvalidResponse(_))
        ));
    }
    #[derive(CandidType)]
    enum FutureError {
        Unknown,
    }
    let bytes = candid::encode_one(Result::<RemoteAttempt, _>::Err(FutureError::Unknown)).unwrap();
    assert!(matches!(
        decode_response::<RemoteAttempt>(&bytes),
        Err(PublishError::InvalidResponse(_))
    ));
}

#[cfg(unix)]
struct FakeProcess {
    root: PathBuf,
    cli: IcpCli,
}

#[cfg(unix)]
impl FakeProcess {
    fn new(stdout: &str, stderr: &str, exit: u8, local_checked: bool) -> Self {
        let root = crate::temporary_workspace().unwrap();
        std::fs::write(root.join("stdout"), format!("{stdout}\n")).unwrap();
        std::fs::write(root.join("stderr"), format!("{stderr}\n")).unwrap();
        std::fs::write(root.join("status"), format!("{exit}\n")).unwrap();
        let mut cli = IcpCli::new("agent-court-local").unwrap();
        // The executable is immutable during tests. Creating and immediately
        // executing a script can race a concurrent fork inheriting its write FD
        // (ETXTBSY), even with close-on-exec set.
        cli.executable = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/icp.sh");
        cli.project_root = root.clone();
        cli.local_checked = local_checked;
        cli.interactive = false;
        Self { root, cli }
    }
}

#[cfg(unix)]
impl Drop for FakeProcess {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
#[cfg(unix)]
fn process_exit_zero_with_candid_err_is_not_success() {
    let bytes =
        candid::encode_one(Result::<RemoteAttempt, _>::Err(RegistryError::Unauthorized)).unwrap();
    let mut fake = FakeProcess::new(&hex::encode(bytes), "", 0, true);
    // Exercise the real subprocess + hex + Candid decoder path.
    let result: Result<RemoteAttempt, _> =
        fake.cli
            .call("register_attempt", candid::encode_args(()), false);
    assert!(matches!(
        result,
        Err(PublishError::Registry(RegistryError::Unauthorized))
    ));
}

#[test]
#[cfg(unix)]
fn process_failures_distinguish_network_expiry_and_locked_identity() {
    for (message, kind) in [
        ("connection refused", CliFailure::Connection),
        ("request timed out", CliFailure::Connection),
        ("Invalid request expiry", CliFailure::InvalidExpiry),
        (
            "failed to read password: not a terminal",
            CliFailure::IdentityUnavailable,
        ),
        ("identity not found", CliFailure::IdentityUnavailable),
        (
            "failed to decrypt identity",
            CliFailure::IdentityUnavailable,
        ),
        ("unknown failure", CliFailure::Process),
    ] {
        let mut fake = FakeProcess::new("", message, 1, true);
        let result = fake.cli.get_attempt("a");
        assert!(
            matches!(result, Err(PublishError::Cli(actual)) if actual == kind),
            "diagnostic {message:?}: expected {kind:?}, got {result:?}"
        );
    }
    let mut cli = IcpCli::new("agent-court-local").unwrap();
    cli.executable = "/this-test-executable-does-not-exist".into();
    assert!(matches!(
        cli.get_attempt("a"),
        Err(PublishError::Cli(CliFailure::Process))
    ));
}

#[test]
#[cfg(unix)]
fn process_output_requires_valid_hex_and_expected_candid() {
    let ok = candid::encode_one(Result::<Option<Mission>, RegistryError>::Ok(None)).unwrap();
    let mut fake = FakeProcess::new(&hex::encode(ok), "", 0, true);
    assert!(fake.cli.get_mission("counter-v1").unwrap().is_none());
    for output in ["", "(variant { Ok = null })", "xyz", "4449444c"] {
        let mut fake = FakeProcess::new(output, "", 0, true);
        assert!(matches!(
            fake.cli.get_mission("counter-v1"),
            Err(PublishError::InvalidResponse(_))
        ));
    }
}

#[test]
#[cfg(unix)]
fn preflight_rejects_public_or_unmanaged_networks_before_any_canister_call() {
    for network in [
        r#"{"managed":false,"api_url":"https://icp-api.io/"}"#,
        r#"{"managed":true,"api_url":"https://icp-api.io/"}"#,
        r#"{"managed":false,"api_url":"http://127.0.0.1:8000/"}"#,
        r#"{"managed":true,"api_url":"http://127.0.0.1:8000@evil.test/"}"#,
    ] {
        let mut fake = FakeProcess::new(network, "", 0, false);
        assert!(matches!(
            fake.cli.ensure_local(),
            Err(PublishError::Validation(_))
        ));
    }
    let mut fake = FakeProcess::new(
        r#"{"managed":true,"api_url":"http://127.0.0.1:8000/"}"#,
        "",
        0,
        false,
    );
    fake.cli.ensure_local().unwrap();
    assert!(fake.cli.local_checked);
}

/// Explicit opt-in only. Never calls register_attempt or any update method.
#[test]
#[ignore = "requires the existing local ICP network; read-only queries only"]
fn read_only_local_icp() {
    let mut cli = IcpCli::new("anonymous").unwrap();
    let mission = cli
        .get_mission("counter-v1")
        .unwrap()
        .expect("counter-v1 must already exist");
    assert_eq!(
        mission.criteria_hash,
        crate::sha256(crate::CRITERIA.as_bytes())
    );
    assert!(cli
        .get_attempt("stage4-read-only-absent")
        .unwrap()
        .is_none());
    let attempts = cli.list_attempts("counter-v1", None).unwrap();
    println!("ICP local leído mediante Rust + icp-cli: counter-v1, {} intentos en la primera página; sin escrituras.", attempts.len());
}
