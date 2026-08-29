#!/usr/bin/env bash
# Functional CI boundary invoked by the required aggregate workflow job.
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
#   the skipped branch is asserted, not ignored. An unselected owner must
#   *skip* its job; accepting "not failed" would let a silently-cancelled job
#   satisfy the gate that exists to require it.
set -euo pipefail

for required in SCOPE_RESULT FAST_GATE_RESULT TEST_LINUX_RESULT TEST_MACOS_RESULT \
    TEST_INSTALL_RESULT DOCS_BUILD_RESULT SITE_BUILD_RESULT \
    RELEASE_SITE_BUILD_RESULT; do
    # An unset result is the dangerous case: `test "" = success` fails, but
    # only by accident, and a renamed job would fail here for a reason nobody
    # could read. Named explicitly instead.
    : "${!required:?job result $required was not passed to the gate}"
done

for selector in CI_OWNERS TEST_LINUX_REQUIRED TEST_MACOS_REQUIRED \
    TEST_INSTALL_REQUIRED DOCS_BUILD_REQUIRED SITE_BUILD_REQUIRED \
    RELEASE_SITE_BUILD_REQUIRED; do
    : "${!selector:?CI owner selection $selector was not passed to the gate}"
done

echo "owners:             $CI_OWNERS"
echo "scope:              $SCOPE_RESULT"
echo "fast-gate:          $FAST_GATE_RESULT"
echo "test-linux:         $TEST_LINUX_RESULT"
echo "test:               $TEST_MACOS_RESULT"
echo "test-install:       $TEST_INSTALL_RESULT"
echo "docs-build:         $DOCS_BUILD_RESULT"
echo "site-build:         $SITE_BUILD_RESULT"
echo "release-site-build: $RELEASE_SITE_BUILD_RESULT"

test "$SCOPE_RESULT" = success
test "$FAST_GATE_RESULT" = success

if [ "$TEST_LINUX_REQUIRED" = true ]; then
    test "$TEST_LINUX_RESULT" = success
else
    test "$TEST_LINUX_RESULT" = skipped
fi

if [ "$TEST_MACOS_REQUIRED" = true ]; then
    test "$TEST_MACOS_RESULT" = success
else
    test "$TEST_MACOS_RESULT" = skipped
fi

if [ "$TEST_INSTALL_REQUIRED" = true ]; then
    test "$TEST_INSTALL_RESULT" = success
else
    test "$TEST_INSTALL_RESULT" = skipped
fi

if [ "$DOCS_BUILD_REQUIRED" = true ]; then
    test "$DOCS_BUILD_RESULT" = success
else
    test "$DOCS_BUILD_RESULT" = skipped
fi

if [ "$SITE_BUILD_REQUIRED" = true ]; then
    test "$SITE_BUILD_RESULT" = success
else
    test "$SITE_BUILD_RESULT" = skipped
fi

if [ "$RELEASE_SITE_BUILD_REQUIRED" = true ]; then
    test "$RELEASE_SITE_BUILD_RESULT" = success
else
    test "$RELEASE_SITE_BUILD_RESULT" = skipped
fi
