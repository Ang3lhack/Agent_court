use std::path::PathBuf;

pub const USAGE: &str = "usage: agent-court-evaluator [--evidence-dir <directory>] [verify <attempt_id> | publish <attempt_id> [--identity <name>]]\n       agent-court-evaluator agent <builder|reviewer|repair> <working-directory> <prompt>";

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Evaluate,
    Verify(String),
    Publish {
        attempt_id: String,
        identity: String,
    },
    Agent(Vec<String>),
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
            _ => return Err(USAGE),
        };
        Ok(Self {
            evidence_dir,
            action,
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
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }
}
