# Evidence registry canister

This Rust canister stores mission metadata and evaluation attempt metadata. It
does not execute code and does not store source files or full test reports.

## Interface

- `register_mission`: creates a mission once, with a SHA-256 hash of its criteria.
- `register_attempt`: appends an attempt after checking its mission, hashes,
  unique ID, and `previous_attempt_id` against the current history tip.
- `get_mission`, `list_missions`, `get_attempt`, `list_attempts`: public queries.
- Attempt and mission entries are stored in `ic-stable-structures` and survive
  canister upgrades. No method edits or deletes an existing entry.

All update methods require the caller to be a canister controller. Queries are
public. Controller access follows the canister's controller settings, so protect
those principals and add a trusted backup controller before any public deployment.
This is an append-only application interface, not absolute immutability: a
controller can upgrade the canister or change its controllers. A SHA-256 hash
identifies the bytes submitted; it does not prove the code is correct.

The canister accepts the controller's claimed verdict; it does not rerun Docker,
recompute file digests, authenticate agent logs, or attest execution. If the
anonymous principal were made a controller, its access would be shared by
everyone. The MVP does not change controllers or identities automatically.

An attempt stores the eight original fields: `mission_id`, `attempt_id`,
`previous_attempt_id`, `criteria_hash`, `artifact_hash`, `report_hash`, `verdict`,
and `timestamp_unix_ms`. It adds `registered_by` and `registered_at` (canister
time in nanoseconds; the supplied evaluation timestamp is milliseconds).
Source files, report bodies, manifests and agent logs remain local. Ordinary
queries do not provide application-level certification of those local files.

Inputs are bounded (100 missions, 10,000 attempts, 512-byte mission descriptions,
and pages of at most 100 records). The current history is a hackathon-scale MVP;
large histories need an indexed pagination design.

## Local build and existing-network check

The presentation uses the existing backend and history. These commands do not
deploy or publish:

```sh
cargo test --manifest-path backend/Cargo.toml --locked
icp build backend --environment local
icp network status --environment local --json
icp canister call backend list_missions '(null, 20 : nat16)' \
  --environment local --identity anonymous --query --output candid --candid backend/backend.did
```

Tests use host-side test data; `icp build` compiles but does not deploy. They are
operator commands, not a claim of a new test/deployment run in stage 7.

Initial creation in a separate, disposable local environment is a manual step:
`icp network start --environment local -d`, then
`icp deploy backend --environment local --identity EXISTING_CONTROLLER_IDENTITY`.
It requires an existing identity selected by the operator; do not use it to
repair the current presentation environment. The official local-development
guide describes stop/start as discarding local network state. Stable structures
support compatible upgrades, not reinstallations or network loss.

Use only `local` here. Mainnet operation and production controller governance
are outside this MVP. See the [hackathon guide and manual publication procedure](../README.md),
[Rust CDK](https://docs.internetcomputer.org/languages/rust/),
[stable structures](https://docs.internetcomputer.org/languages/rust/stable-structures/),
and [ICP CLI local-development guide](https://cli.internetcomputer.org/1.7/guides/local-development.md).
