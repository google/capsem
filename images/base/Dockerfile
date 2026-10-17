# The Capsem base image. Every official image (dev, codex-cli, claude-code,
# agy) builds FROM this one by digest, so its layers are stored and pulled
# once. It carries what every agent session needs and nothing agent-specific:
# Debian, CA trust, git, Python with uv, Node, Ollama on CPU, and the
# unprivileged user agents run as.
FROM debian:bookworm-slim@sha256:3783cc01769c7b2b1b83a5c5ad96c815348e28ed7da68e2e3687004faa906251

ARG TARGETARCH
# Debian packages come from one dated snapshot, so a rebuild resolves the same
# package versions. Plain http: the slim image has no CA bundle yet, and apt
# verifies every package against Debian's signing keys regardless.
ARG DEBIAN_SNAPSHOT=20260810T000000Z

RUN set -eux; \
    rm -f /etc/apt/sources.list.d/debian.sources; \
    for suite in bookworm bookworm-updates; do \
        echo "deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/$DEBIAN_SNAPSHOT $suite main"; \
    done > /etc/apt/sources.list; \
    echo "deb [check-valid-until=no] http://snapshot.debian.org/archive/debian-security/$DEBIAN_SNAPSHOT bookworm-security main" \
        >> /etc/apt/sources.list; \
    apt-get -o Acquire::Check-Valid-Until=false update; \
    apt-get install -y --no-install-recommends \
        bash ca-certificates curl git jq less openssh-client procps \
        python3 python3-venv ripgrep tar xz-utils zstd; \
    rm -rf /var/lib/apt/lists/*

COPY install-tools.sh /tmp/install-tools.sh
RUN sh /tmp/install-tools.sh "$TARGETARCH" && rm /tmp/install-tools.sh

# Agents refuse to skip their permission prompts as root, and nothing in a
# session needs it. Container root still owns the workspace, which is mounted
# at /workspace.
RUN useradd --create-home --uid 1000 --shell /bin/bash capsem
USER capsem
WORKDIR /workspace
ENV PATH=/home/capsem/.local/bin:/usr/local/bin:/usr/bin:/bin \
    OLLAMA_HOST=127.0.0.1:11434
CMD ["/bin/bash"]
