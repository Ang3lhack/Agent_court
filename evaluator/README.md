# Agent Court evaluator (stage 1)

Run the controlled counter mission from the repository root:

```sh
cargo run --manifest-path evaluator/Cargo.toml --offline
```

The evaluator writes a JSON report with a verdict, per-check status, hashes for
the criteria, exact built-in fixture, and report payload, plus a Unix timestamp.
It also appends each attempt to `.agent-court/evidence.jsonl`, linking it to the
previous attempt. The local log is append-only by this program, but is not
tamper-proof.

It compiles and executes three real Rust tests in a unique temporary directory.
The only executed source is the trusted fixture embedded in `src/main.rs`; this
stage does not execute agent-generated code and is not an OS sandbox.

Run the evaluator's own tests with:

```sh
cargo test --manifest-path evaluator/Cargo.toml --offline
```
