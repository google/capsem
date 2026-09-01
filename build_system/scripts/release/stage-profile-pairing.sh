#!/usr/bin/env bash
# Stage the exact profile and pulled-binary pairing a profile release tests.
#
# Lifted verbatim out of release-assets.yaml:test-profile-pairing, which was
# fifty-four executable lines of YAML: four scripts, a manifest comparison and
# sixteen environment writes that no test could call and no linter read. The
# commands and their order are unchanged; the GitHub expressions the step used
# inline arrive as environment instead, so this runs the same way from a shell.
set -euo pipefail

: "${PUBLICATION_IDENTITY:?the authored publication identity is required}"
: "${GITHUB_REPOSITORY:?the owning repository is required}"
: "${RELEASE_CHANNEL:?the channel under test is required}"
: "${RELEASE_BASELINE_CHANNEL:?the verified baseline channel is required}"
: "${RELEASE_PROFILE:?the profile under test is required}"
: "${ACTIVATION_READY:?the authored profile activation decision is required}"
: "${GITHUB_ENV:?this stages the environment a later step reads}"

if [[ "$ACTIVATION_READY" != "true" && "$ACTIVATION_READY" != "false" ]]; then
    echo "profile activation decision must be true or false" >&2
    exit 1
fi

PUBLICATION_BASE="https://github.com/${GITHUB_REPOSITORY}/releases/download/${PUBLICATION_IDENTITY}"
PUBLICATION_DIR="cache/target/asset-release/${PUBLICATION_IDENTITY}"

# The public-before pair must agree before anything is pulled against it.
uv run --project build_system --frozen python build_system/scripts/release/verify-release-inputs.py \
    --input-dir cache/target/profile-public-before/packages
uv run --project build_system --frozen python build_system/scripts/release/verify-release-inputs.py \
    --input-dir cache/target/profile-public-before/profiles
cmp \
    cache/target/profile-public-before/packages/manifest.json \
    cache/target/profile-public-before/profiles/manifest.json

uv run --project build_system --frozen python build_system/scripts/release/fetch-release-artifacts.py \
    --manifest-url "file://$PWD/cache/target/source-channel/manifest.json" \
    --kind profiles \
    --output cache/target/candidate-profile-inputs \
    --local-publication-base "$PUBLICATION_BASE" \
    --local-publication-dir "$PUBLICATION_DIR"
uv run --project build_system --frozen python build_system/scripts/release/verify-release-inputs.py \
    --input-dir cache/target/candidate-profile-inputs

# A retired or first-channel public-before graph deliberately has no package.
# The profile still owes its own digest and KVM boot proof, but that is not a
# complete package/profile pairing and must not be made to look like one with
# a placeholder package. The workflow invokes the dedicated private artifact
# module after this shared input verification and withholds activation.
if [[ "$ACTIVATION_READY" == "false" ]]; then
    exit 0
fi
[[ "$ACTIVATION_READY" == "true" ]]

uv run --project build_system --frozen python build_system/scripts/release/stage-release-test-inputs.py \
    --input-dir cache/target/profile-public-before/packages \
    --binary-dir cache/target/cargo/debug
uv run --project build_system --frozen python build_system/scripts/release/stage-release-test-inputs.py \
    --input-dir cache/target/candidate-profile-inputs \
    --assets-dir assets \
    --config-root cache/target/release/staging/config \
    --shared-config-root config

CAPSEM_ASSET_MANIFEST="$PWD/cache/target/assets/manifest.json" \
CAPSEM_CONFIG_ROOT="$PWD/cache/target/release/staging/config" \
CAPSEM_CONFIG_OUTPUT_ROOT="$PWD/cache/target/config" \
    bash build_system/scripts/build/materialize-config.sh --pair-content

package=$(uv run --project build_system --frozen python build_system/scripts/release/stage-release-test-inputs.py \
    --input-dir cache/target/profile-public-before/packages \
    --print-package-path)
test -n "$package"
uv run --project build_system --frozen python build_system/packaging/linux/install-deb-runtime-dependencies.py "$package" --config config/gate.toml

{
    echo "CAPSEM_RELEASE_PACKAGE=$PWD/$package"
    echo "CAPSEM_RELEASE_BIN_DIR=$PWD/cache/target/cargo/debug"
    echo "CAPSEM_RELEASE_INPUT_DIR=$PWD/cache/target/candidate-profile-inputs"
    echo "CAPSEM_RELEASE_CHANNEL=${RELEASE_CHANNEL}"
    echo "CAPSEM_RELEASE_BASELINE_CHANNEL=${RELEASE_BASELINE_CHANNEL}"
    echo "CAPSEM_RELEASE_TRANSITION=auto"
    echo "CAPSEM_RELEASE_BEFORE_MANIFEST=$PWD/cache/target/profile-public-before/profiles/manifest.json"
    echo "CAPSEM_RELEASE_AFTER_MANIFEST=$PWD/cache/target/source-channel/manifest.json"
    echo "CAPSEM_RELEASE_BEFORE_PACKAGE=$PWD/$package"
    echo "CAPSEM_RELEASE_BEFORE_PROFILE_INPUTS=$PWD/cache/target/profile-public-before/profiles"
    echo "CAPSEM_RELEASE_AFTER_PROFILE_INPUTS=$PWD/cache/target/candidate-profile-inputs"
    echo "CAPSEM_RELEASE_PROFILE=${RELEASE_PROFILE}"
    echo "CAPSEM_RELEASE_CANDIDATE_PROFILE_PUBLICATION=$PWD/$PUBLICATION_DIR"
    echo "CAPSEM_RELEASE_PUBLICATION_BASE=$PUBLICATION_BASE"
    echo "CAPSEM_TEST_BINARY=$PWD/cache/target/cargo/debug/capsem"
    echo "CAPSEM_TEST_ASSETS_DIR=$PWD/cache/target/assets"
    echo "CAPSEM_TEST_CONFIG_ROOT=$PWD/cache/target/config"
} >> "$GITHUB_ENV"
