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

Inputs are bounded (100 missions, 10,000 attempts, 512-byte mission descriptions,
and pages of at most 100 records). The current history is a hackathon-scale MVP;
large histories need an indexed pagination design.

## Local build and replica check

```sh
cargo test --manifest-path backend/Cargo.toml --offline
icp build backend
icp network start -d
icp deploy backend
```

Use only the `local` environment for this flow. Do not deploy to the public IC
until local update and query calls have been verified and the controller policy
is reviewed.
