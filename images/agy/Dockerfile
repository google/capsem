# Antigravity (agy) on the Capsem base. BASE is the base image by digest. The
# session is the sandbox, so agy runs without its own permission prompts.
ARG BASE
FROM ${BASE}
ARG TARGETARCH
ARG AGY_VERSION=1.1.3
USER root
RUN set -eu; \
    case "$TARGETARCH" in \
        arm64) asset=agy_cli_linux_arm64.tar.gz sha256=453f9c5530877ab6369e2536e576cfab2bbbcb45923a9bc776678142538e419d ;; \
        amd64) asset=agy_cli_linux_x64.tar.gz sha256=7a7239a69b65d3cf3af7e75f27b2ff4e9cce696a7b9a9e5c37c695f1c74eec34 ;; \
        *) echo "unsupported architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    work="$(mktemp -d)"; \
    curl -fsSL "https://github.com/google-antigravity/antigravity-cli/releases/download/$AGY_VERSION/$asset" -o "$work/$asset"; \
    echo "$sha256  $work/$asset" | sha256sum -c -; \
    tar -xzf "$work/$asset" -C "$work"; \
    install -m 555 "$work/antigravity" /usr/local/bin/agy-real; \
    rm -rf "$work"; \
    printf '#!/bin/sh\nexec /usr/local/bin/agy-real --dangerously-skip-permissions "$@"\n' > /usr/local/bin/agy; \
    chmod 555 /usr/local/bin/agy
USER capsem
CMD ["agy"]
