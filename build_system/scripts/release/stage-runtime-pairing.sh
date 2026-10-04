#!/usr/bin/env bash
# Stage the exact runtime and pulled-binary pairing a runtime release tests.
#
# Lifted out of release-assets.yaml:test-runtime-pairing so a test can call it
# and ShellCheck can read it. The GitHub expressions the step would have used
# inline arrive as environment instead, so this runs the same way from a shell.
set -euo pipefail

: "${PUBLICATION_IDENTITY:?the authored publication identity is required}"
: "${GITHUB_REPOSITORY:?the owning repository is required}"
: "${RELEASE_CHANNEL:?the channel under test is required}"
: "${RELEASE_BASELINE_CHANNEL:?the verified baseline channel is required}"
: "${ACTIVATION_READY:?the authored runtime activation decision is required}"
: "${GITHUB_ENV:?this stages the environment a later step reads}"

if [[ "$ACTIVATION_READY" != "true" && "$ACTIVATION_READY" != "false" ]]; then
    echo "runtime activation decision must be true or false" >&2
    exit 1
fi

PUBLICATION_BASE="https://github.com/${GITHUB_REPOSITORY}/releases/download/${PUBLICATION_IDENTITY}"
PUBLICATION_DIR="cache/target/asset-release/${PUBLICATION_IDENTITY}"

# The public-before pair must agree before anything is pulled against it.
uv run --project build_system --frozen python build_system/scripts/release/verify-release-inputs.py \
    --input-dir cache/target/runtime-public-before/packages
uv run --project build_system --frozen python build_system/scripts/release/verify-release-inputs.py \
    --input-dir cache/target/runtime-public-before/runtime
cmp \
    cache/target/runtime-public-before/packages/manifest.json \
    cache/target/runtime-public-before/runtime/manifest.json

uv run --project build_system --frozen python build_system/scripts/release/fetch-release-artifacts.py \
    --manifest-url "file://$PWD/cache/target/source-channel/manifest.json" \
    --kind runtime \
    --output cache/target/candidate-runtime-inputs \
    --local-publication-base "$PUBLICATION_BASE" \
    --local-publication-dir "$PUBLICATION_DIR"
uv run --project build_system --frozen python build_system/scripts/release/verify-release-inputs.py \
    --input-dir cache/target/candidate-runtime-inputs

# A retired or first-channel public-before graph deliberately has no package.
# The runtime still owes its own digest and KVM boot proof, but that is not a
# complete package/runtime pairing and must not be made to look like one with
# a placeholder package. The lane proves the runtime alone and withholds
# activation.
if [[ "$ACTIVATION_READY" == "false" ]]; then
    exit 0
fi

uv run --project build_system --frozen python build_system/scripts/release/stage-release-test-inputs.py \
    --input-dir cache/target/candidate-runtime-inputs \
    --assets-dir cache/target/assets

# The service configuration is the checkout's: a runtime publishes no config.
CAPSEM_ASSET_MANIFEST="$PWD/cache/target/assets/manifest.json" \
CAPSEM_CONFIG_OUTPUT_ROOT="$PWD/cache/target/config" \
    bash build_system/scripts/build/materialize-config.sh --pair-content

# Materialization builds its admin tool; restore the exact pulled cohort last.
uv run --project build_system --frozen python build_system/scripts/release/stage-release-test-inputs.py \
    --input-dir cache/target/runtime-public-before/packages \
    --binary-dir cache/target/cargo/debug

package=$(uv run --project build_system --frozen python build_system/scripts/release/stage-release-test-inputs.py \
    --input-dir cache/target/runtime-public-before/packages \
    --print-package-path)
test -n "$package"
uv run --project build_system --frozen python build_system/packaging/linux/install-deb-runtime-dependencies.py "$package" --config config/gate.toml \
    --package-inputs cache/target/runtime-public-before/packages

{
    echo "CAPSEM_RELEASE_PACKAGE=$PWD/$package"
    echo "CAPSEM_RELEASE_BIN_DIR=$PWD/cache/target/cargo/debug"
    echo "CAPSEM_RELEASE_INPUT_DIR=$PWD/cache/target/candidate-runtime-inputs"
    echo "CAPSEM_RELEASE_CHANNEL=${RELEASE_CHANNEL}"
    echo "CAPSEM_RELEASE_BASELINE_CHANNEL=${RELEASE_BASELINE_CHANNEL}"
    echo "CAPSEM_RELEASE_TRANSITION=auto"
    echo "CAPSEM_RELEASE_BEFORE_MANIFEST=$PWD/cache/target/runtime-public-before/runtime/manifest.json"
    echo "CAPSEM_RELEASE_AFTER_MANIFEST=$PWD/cache/target/source-channel/manifest.json"
    echo "CAPSEM_RELEASE_BEFORE_PACKAGE=$PWD/$package"
    echo "CAPSEM_RELEASE_BEFORE_INPUTS=$PWD/cache/target/runtime-public-before/runtime"
    echo "CAPSEM_RELEASE_AFTER_INPUTS=$PWD/cache/target/candidate-runtime-inputs"
    echo "CAPSEM_RELEASE_RUNTIME=1"
    echo "CAPSEM_RELEASE_CANDIDATE_RUNTIME_PUBLICATION=$PWD/$PUBLICATION_DIR"
    echo "CAPSEM_RELEASE_PUBLICATION_BASE=$PUBLICATION_BASE"
    echo "CAPSEM_TEST_BINARY=$PWD/cache/target/cargo/debug/capsem"
    echo "CAPSEM_TEST_ASSETS_DIR=$PWD/cache/target/assets"
    echo "CAPSEM_TEST_CONFIG_ROOT=$PWD/cache/target/config"
} >> "$GITHUB_ENV"
