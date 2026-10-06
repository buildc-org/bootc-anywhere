#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"

if [ "${1:-}" = "--fix" ]; then
    cargo fmt
    echo "lint: formatted the source"
    exit
fi

mapfile -t crate_src < <(find . -path ./target -prune -o -name '*.rs' -type f -print)

if grep -nE '\buse +(::)?tect\b|\bextern +crate +tect\b|(^|[^A-Za-z0-9_:])tect::' "${crate_src[@]}"; then
    echo "lint: the installer naming the tect crate, which it does not depend on" >&2
    exit 1
fi

if grep -nE 'Command::new\("[^"]*tect"|"/usr/bin/tect"' "${crate_src[@]}"; then
    echo "lint: the installer running the tect binary, which media may not carry" >&2
    exit 1
fi

# `src` only: the goldens' comments name ratatui where they say why a drawn
# frame arrives in fragments, and prose is not a dependency.
if grep -rn 'ratatui' src; then
    echo "lint: ratatui reached directly, and it belongs to the common crate" >&2
    exit 1
fi

cargo fmt --check || {
    echo "lint: unformatted, run ./lint.sh --fix" >&2
    exit 1
}
echo "lint: the source is clean"

cargo deny --locked check bans licenses sources
echo "lint: the dependency policy accepts the locked graph"

cargo nextest run --locked
echo "lint: the installer does what it did"

cargo build --quiet
if strings target/debug/bootc-anywhere | grep -n '{PROGRAM}'; then
    echo "lint: a message carries {PROGRAM} uninterpolated, so it is not in a format!" >&2
    exit 1
fi
echo "lint: every message names the program rather than the placeholder"
