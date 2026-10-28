use std::path::PathBuf;

pub const USAGE: &str = "usage: agent-court-evaluator [--evidence-dir <directory>] [evaluate-file <path> [--no-run] | verify <attempt_id> | publish <attempt_id> [--identity <name>]]\n       agent-court-evaluator [--evidence-dir <directory>] orchestrate [counter-v1] [--evidence-dir <directory>] [--demo-seed <candidate.rs> | --publish [--identity <name>]]\n       agent-court-evaluator agent <builder|reviewer|repair> <working-directory> <prompt>";

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Evaluate,
    EvaluateFile {
        path: PathBuf,
        no_run: bool,
    },
    Verify(String),
    Publish {
        attempt_id: String,
        identity: String,
    },
    Agent(Vec<String>),
    Orchestrate {
        publish: bool,
        identity: String,
        demo_seed: Option<PathBuf>,
    },
    Help,
}

#[derive(Debug)]
pub struct Cli {
    pub evidence_dir: PathBuf,
    pub action: Action,
}

impl Cli {
    pub fn parse(args: &[String]) -> Result<Self, &'static str> {
        let (evidence_dir, rest, explicit_store) = match args {
            [flag, directory, rest @ ..] if flag == "--evidence-dir" => {
                if directory.is_empty() || directory.starts_with('-') {
                    return Err(USAGE);
                }
                (PathBuf::from(directory), rest, true)
            }
            _ => (PathBuf::from(".agent-court"), args, false),
        };
        let action = match rest {
            [] => Action::Evaluate,
            [command, path] if command == "evaluate-file" => Action::EvaluateFile {
                path: path.into(),
                no_run: false,
            },
            [command, path, flag] if command == "evaluate-file" && flag == "--no-run" => {
                Action::EvaluateFile {
                    path: path.into(),
                    no_run: true,
                }
            }
            [command] if command == "--help" || command == "-h" => Action::Help,
            [command, id] if command == "verify" => Action::Verify(id.clone()),
            [command, id] if command == "publish" => Action::Publish {
                attempt_id: id.clone(),
                identity: "agent-court-local".into(),
            },
            [command, id, flag, identity] if command == "publish" && flag == "--identity" => {
                Action::Publish {
                    attempt_id: id.clone(),
                    identity: identity.clone(),
                }
            }
            [command, rest @ ..] if command == "agent" && !explicit_store => {
                Action::Agent(rest.to_vec())
            }
            [command, rest @ ..] if command == "orchestrate" => {
                return Self::orchestrate(evidence_dir, explicit_store, rest);
            }
            _ => return Err(USAGE),
        };
        Ok(Self {
            evidence_dir,
            action,
        })
    }

    fn orchestrate(
        mut evidence_dir: PathBuf,
        mut explicit_store: bool,
        args: &[String],
    ) -> Result<Self, &'static str> {
        let mut rest = if args.first().is_some_and(|arg| arg == "counter-v1") {
            &args[1..]
        } else {
            args
        };
        let mut publish = false;
        let mut identity = None;
        let mut demo_seed = None;
        while !rest.is_empty() {
            rest = match rest {
                [flag, tail @ ..] if flag == "--publish" && !publish => {
                    publish = true;
                    tail
                }
                [flag, value, tail @ ..]
                    if flag == "--demo-seed"
                        && demo_seed.is_none()
                        && !value.is_empty()
                        && !value.starts_with('-') =>
                {
                    demo_seed = Some(PathBuf::from(value));
                    tail
                }
                [flag, value, tail @ ..]
                    if flag == "--identity"
                        && identity.is_none()
                        && !value.is_empty()
                        && !value.starts_with('-') =>
                {
                    identity = Some(value.clone());
                    tail
                }
                [flag, value, tail @ ..]
                    if flag == "--evidence-dir"
                        && !explicit_store
                        && !value.is_empty()
                        && !value.starts_with('-') =>
                {
                    evidence_dir = value.into();
                    explicit_store = true;
                    tail
                }
                _ => return Err(USAGE),
            };
        }
        if demo_seed.is_some() && publish {
            return Err("--demo-seed no permite --publish; publica después de comprobar la cadena manualmente");
        }
        if identity
            .as_deref()
            .is_some_and(|id| !crate::evidence::valid_id(id))
            || (identity.is_some() && !publish)
        {
            return Err(USAGE);
        }
        Ok(Self {
            evidence_dir,
            action: Action::Orchestrate {
                publish,
                identity: identity.unwrap_or_else(|| "agent-court-local".into()),
                demo_seed,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, &'static str> {
        Cli::parse(&args.iter().map(|s| (*s).into()).collect::<Vec<_>>())
    }

    #[test]
    fn default_and_independent_store_work_for_all_evidence_commands() {
        assert_eq!(
            parse(&[]).unwrap().evidence_dir,
            PathBuf::from(".agent-court")
        );
        for tail in [
            vec![],
            vec!["verify", "attempt-1"],
            vec!["publish", "attempt-1"],
            vec!["evaluate-file", "candidate.rs"],
        ] {
            let mut args = vec!["--evidence-dir", ".agent-court/demo-stage4"];
            args.extend(tail);
            assert_eq!(
                parse(&args).unwrap().evidence_dir,
                PathBuf::from(".agent-court/demo-stage4")
            );
        }
        assert_eq!(
            parse(&["publish", "a", "--identity", "admin"])
                .unwrap()
                .action,
            Action::Publish {
                attempt_id: "a".into(),
                identity: "admin".into()
            }
        );
    }

    #[test]
    fn agent_arguments_are_preserved_including_option_like_prompt_text() {
        assert_eq!(
            parse(&["agent", "reviewer", ".", "--evidence-dir", "literal"])
                .unwrap()
                .action,
            Action::Agent(vec![
                "reviewer".into(),
                ".".into(),
                "--evidence-dir".into(),
                "literal".into()
            ])
        );
    }

    #[test]
    fn invalid_usage_cannot_accidentally_generate_evidence() {
        for args in [
            vec!["--evidence-dir"],
            vec!["--evidence-dir", ""],
            vec!["publish"],
            vec!["verify"],
            vec!["publish", "a", "--network", "ic"],
            vec!["unknown"],
            vec!["verify", "a", "--evidence-dir", "elsewhere"],
            vec!["--evidence-dir", "x", "agent"],
            vec!["evaluate-file"],
            vec!["evaluate-file", "candidate.rs", "--unknown"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn orchestrate_defaults_to_real_agents_local_evidence_and_explicit_publication() {
        let local = Action::Orchestrate {
            publish: false,
            identity: "agent-court-local".into(),
            demo_seed: None,
        };
        assert_eq!(parse(&["orchestrate"]).unwrap().action, local);
        assert_eq!(parse(&["orchestrate", "counter-v1"]).unwrap().action, local);
        for args in [
            vec!["--evidence-dir", "own-store", "orchestrate", "counter-v1"],
            vec!["orchestrate", "--evidence-dir", "own-store"],
        ] {
            let cli = parse(&args).unwrap();
            assert_eq!(cli.evidence_dir, PathBuf::from("own-store"));
            assert_eq!(cli.action, local);
        }
        assert_eq!(
            parse(&[
                "orchestrate",
                "counter-v1",
                "--publish",
                "--identity",
                "supervised"
            ])
            .unwrap()
            .action,
            Action::Orchestrate {
                publish: true,
                identity: "supervised".into(),
                demo_seed: None,
            }
        );
        for args in [
            vec!["orchestrate", "counter-v2"],
            vec!["orchestrate", "--identity", "controller"],
            vec!["orchestrate", "--publish", "--identity", "../bad"],
            vec!["orchestrate", "--publish", "--publish"],
            vec!["orchestrate", "--no-run"],
            vec!["orchestrate", "--evidence-dir"],
            vec!["--evidence-dir", "a", "orchestrate", "--evidence-dir", "b"],
            vec!["orchestrate", "--max-attempts", "3"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn demo_seed_is_explicit_local_and_cannot_be_combined_with_publication() {
        for args in [
            vec![
                "--evidence-dir",
                "own-store",
                "orchestrate",
                "counter-v1",
                "--demo-seed",
                "provided.rs",
            ],
            vec![
                "orchestrate",
                "counter-v1",
                "--demo-seed",
                "provided.rs",
                "--evidence-dir",
                "own-store",
            ],
        ] {
            let cli = parse(&args).unwrap();
            assert_eq!(cli.evidence_dir, PathBuf::from("own-store"));
            assert_eq!(
                cli.action,
                Action::Orchestrate {
                    publish: false,
                    identity: "agent-court-local".into(),
                    demo_seed: Some("provided.rs".into())
                }
            );
        }
        for args in [
            vec!["orchestrate", "counter-v1", "--demo-seed"],
            vec!["orchestrate", "--demo-seed", ""],
            vec!["orchestrate", "--demo-seed", "--publish"],
            vec!["orchestrate", "--demo-seed", "a.rs", "--demo-seed", "b.rs"],
            vec![
                "orchestrate",
                "--demo-seed",
                "a.rs",
                "--identity",
                "controller",
            ],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
        for args in [
            vec![
                "orchestrate",
                "counter-v1",
                "--demo-seed",
                "a.rs",
                "--publish",
            ],
            vec![
                "orchestrate",
                "--publish",
                "--identity",
                "controller",
                "--demo-seed",
                "a.rs",
            ],
        ] {
            assert!(parse(&args)
                .unwrap_err()
                .contains("--demo-seed no permite --publish"));
        }
    }
}
