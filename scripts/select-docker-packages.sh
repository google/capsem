#!/bin/sh
# Select only the Docker packages missing from an otherwise working host stack.

set -eu

buildx_package() {
    if apt-cache show docker-buildx >/dev/null 2>&1; then
        printf "docker-buildx\n"
    elif apt-cache show docker-buildx-plugin >/dev/null 2>&1; then
        printf "docker-buildx-plugin\n"
    else
        printf "  [FAIL] neither docker-buildx nor docker-buildx-plugin is available from apt\n" >&2
        return 1
    fi
}

# GitHub's Ubuntu images carry Docker CE and containerd.io. Installing Ubuntu's
# docker.io merely because another prerequisite is missing would replace that
# working stack with conflicting packages.
if command -v docker >/dev/null 2>&1 && docker --version >/dev/null 2>&1; then
    if ! docker buildx version >/dev/null 2>&1; then
        buildx_package
    fi
    exit 0
fi

printf "docker.io\n"
buildx_package
