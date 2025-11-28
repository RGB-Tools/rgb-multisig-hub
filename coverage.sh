#!/bin/bash -e
#
# script to run project tests and report code coverage
# uses llvm-cov (https://github.com/taiki-e/cargo-llvm-cov)

LLVM_COV_OPTS=()
CARGO_TEST_OPTS=("--" "--test-threads=1")
COV="cargo llvm-cov --workspace"

_die() {
    echo "err $*"
    exit 1
}

_tit() {
    echo
    echo "========================================"
    echo "$@"
    echo "========================================"
}

help() {
    echo "$NAME [-h|--help] [-t|--test] [--ci] [--ignore-run-fail]"
    echo ""
    echo "options:"
    echo "    -h --help             show this help message"
    echo "    -t --test             only run these test(s)"
    echo "       --ci               run for the CI"
    echo "       --ignore-run-fail  keep running regardless of failure"
}

# cmdline arguments
while [ -n "$1" ]; do
    case $1 in
        -h|--help)
            help
            exit 0
            ;;
        -t|--test)
            CARGO_TEST_OPTS+=("$2")
            shift
            ;;
        --ci)
            $COV --lcov --output-path coverage.lcov \
                "${LLVM_COV_OPTS[@]}" "${CARGO_TEST_OPTS[@]}"
            exit 0
            ;;
        --ignore-run-fail)
            LLVM_COV_OPTS+=("$1")
            ;;
        *)
            help
            _die "unsupported argument \"$1\""
            ;;
    esac
    shift
done

_tit "installing requirements"
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov

_tit "generating coverage report"
IGNORE_PATTERN="/rgb\-multisig\-hub(/.*)?/(tests|examples|benches|migration/src/main.rs|src/database/entities|src/test)($|/)|/rgb\-multisig\-hub/target/llvm\-cov\-target($|/)|^$HOME/\.cargo/(registry|git)/|^$HOME/\.rustup/toolchains($|/)"

$COV --html \
    --ignore-filename-regex "$IGNORE_PATTERN" \
    "${LLVM_COV_OPTS[@]}" "${CARGO_TEST_OPTS[@]}"

## show html report location
echo "generated html report: target/llvm-cov/html/index.html"
