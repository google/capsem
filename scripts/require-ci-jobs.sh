#!/usr/bin/env bash
# The single required status behind branch protection: every CI job that had to
# pass, passed, and every job that had to be skipped was skipped.
#
# Lifted out of ci.yaml:pr-gate, where it was twenty-three executable lines of
# YAML. This is the one step in the repository whose result decides whether a
# PR can merge, and inline it was reachable by no linter and callable by no
# test. Each job result arrives as an environment variable so the workflow
# keeps owning the `needs.<job>.result` expressions.
#
# Two rules this file exists to keep, both held by
# `tests/citadel/test_workflow_enforcement.py`:
#
#   every comparison is a bare command, so its exit status reaches the shell.
#   `test X = success || true`, `test ... ; :`, a trailing `&`, or a pipe into
#   anything all turn a failing gate green, and each reads fine at the call
#   site.
#
#   the skipped branch is asserted, not ignored. A web-only PR must *skip* the
#   expensive jobs; accepting "not failed" would let a silently-cancelled
#   fast-gate satisfy the gate that exists to require it.
set -euo pipefail

for required in FAST_GATE_RESULT TEST_LINUX_RESULT TEST_MACOS_RESULT \
    TEST_INSTALL_RESULT DOCS_BUILD_RESULT SITE_BUILD_RESULT \
    RELEASE_SITE_BUILD_RESULT WEB_ONLY; do
    # An unset result is the dangerous case: `test "" = success` fails, but
    # only by accident, and a renamed job would fail here for a reason nobody
    # could read. Named explicitly instead.
    : "${!required:?job result $required was not passed to the gate}"
done

echo "web-only:           $WEB_ONLY"
echo "fast-gate:          $FAST_GATE_RESULT"
echo "test-linux:         $TEST_LINUX_RESULT"
echo "test:               $TEST_MACOS_RESULT"
echo "test-install:       $TEST_INSTALL_RESULT"
echo "docs-build:         $DOCS_BUILD_RESULT"
echo "site-build:         $SITE_BUILD_RESULT"
echo "release-site-build: $RELEASE_SITE_BUILD_RESULT"

# Both site builds run on every PR, web-only or not.
test "$FAST_GATE_RESULT" = success
test "$DOCS_BUILD_RESULT" = success
test "$SITE_BUILD_RESULT" = success

if [ "$WEB_ONLY" = true ]; then
    test "$TEST_LINUX_RESULT" = skipped
    test "$TEST_MACOS_RESULT" = skipped
    test "$TEST_INSTALL_RESULT" = skipped
    test "$RELEASE_SITE_BUILD_RESULT" = skipped
else
    test "$FAST_GATE_RESULT" = success
    test "$TEST_LINUX_RESULT" = success
    test "$TEST_MACOS_RESULT" = success
    test "$TEST_INSTALL_RESULT" = success
    test "$RELEASE_SITE_BUILD_RESULT" = success
fi
