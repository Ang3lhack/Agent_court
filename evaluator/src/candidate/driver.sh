#!/bin/sh
# Immutable evaluator-owned driver. No candidate data is interpolated as shell.
command -v rustc >/dev/null 2>&1 || exit 20
command -v timeout >/dev/null 2>&1 || exit 20
rustc --version || exit 20
# Validate compiler, linker and executable tmpfs before blaming candidate code.
printf '%s\n' 'fn main() {}' > /work/toolchain-probe.rs || exit 20
timeout -s KILL 5s rustc --edition=2021 /work/toolchain-probe.rs -o /work/toolchain-probe || exit 20
/work/toolchain-probe || exit 20

compile() {
    timeout -s KILL 10s rustc "$@"
    result=$?
    case "$result" in
        0) return 0 ;;
        1) exit 10 ;;               # candidate or public API compilation failure
        124|137) exit 12 ;;          # compilation resource/time limit
        *) exit 20 ;;               # broken compiler/toolchain
    esac
}

compile --edition=2021 --crate-name candidate --crate-type rlib /input/candidate.rs -o /work/libcandidate.rlib
compile --edition=2021 /input/suite.rs --extern candidate=/work/libcandidate.rlib -o /work/checks
timeout -s KILL 5s /work/checks
result=$?
case "$result" in
    73) exit 0 ;;
    124|137) exit 12 ;;
    126|127) exit 20 ;;
    *) exit 11 ;;
esac
