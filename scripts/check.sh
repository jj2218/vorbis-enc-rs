#!/usr/bin/env bash
# The full gate behind `just check`: export guard, format, lint (clippy, docs), then the tests
# against the oracle (aoTuV) and the test items, downloaded on first use.
set -euo pipefail
cd "$(dirname "$0")/.."
just rules fmt-check lint test
