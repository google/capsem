# capsem-debug: everything Capsem's tests need inside a session, so the VM
# runtime rootfs carries none of it -- test runners, the network tools and
# benchmark peers kingslanding drives, package managers, model SDKs and the
# agent CLIs.
#
# All of it is third-party content, every source pinned: Debian from one dated
# snapshot; Node, uv, Ollama, Claude Code and agy by version and SHA-256; the
# npm agent CLIs by lockfile integrity; the Python SDKs by hash. It is a test
# fixture, not an official image: images/catalog.toml does not list it, so no
# default admission policy admits it, and the harness serves it by the digest
# config/gate.toml [functional.debug_image] pins. tests/fixtures/oci/README.md
# says how a maintainer builds, pins and publishes it.
FROM debian:bookworm-slim@sha256:3783cc01769c7b2b1b83a5c5ad96c815348e28ed7da68e2e3687004faa906251

ARG TARGETARCH
# The base image's snapshot, so the two resolve the same package versions.
# Plain http: the slim image has no CA bundle yet, and apt verifies every
# package against Debian's signing keys regardless.
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
        bash ca-certificates coreutils curl diffutils dnsutils file findutils \
        git iperf3 iproute2 iputils-ping jq less netcat-openbsd openssl \
        procps python3 python3-pip python3-pytest python3-rich python3-venv \
        redis-tools socat sqlite3 strace tar wget wrk xz-utils zstd; \
    rm -rf /var/lib/apt/lists/*

COPY install-tools.sh /tmp/install-tools.sh
RUN sh /tmp/install-tools.sh "$TARGETARCH" && rm /tmp/install-tools.sh

# The npm agent CLIs (Codex, Gemini), every transitive package pinned by the
# lockfile's integrity hash. No install scripts: the native addons they would
# build are optional, and the CLIs' platform binaries are ordinary packages.
COPY package.json package-lock.json /opt/capsem-debug/agents/
RUN set -eux; \
    cd /opt/capsem-debug/agents; \
    npm ci --ignore-scripts --no-audit --no-fund; \
    npm cache clean --force; \
    ln -s /opt/capsem-debug/agents/node_modules/.bin/codex /usr/local/bin/codex; \
    ln -s /opt/capsem-debug/agents/node_modules/.bin/gemini /usr/local/bin/gemini

# The model SDKs the client scripts import, in their own environment, every
# distribution pinned by hash (`uv pip compile --generate-hashes` of
# requirements.in). First on PATH, so `python3` has them.
COPY requirements.txt /opt/capsem-debug/requirements.txt
RUN set -eux; \
    uv venv --python /usr/bin/python3 --system-site-packages /opt/capsem-debug/python; \
    uv pip install --python /opt/capsem-debug/python/bin/python --require-hashes --no-deps \
        -r /opt/capsem-debug/requirements.txt; \
    rm -rf /root/.cache

# The same unprivileged user the official images run agents as. The root
# filesystem is read-only in a session, so its home is a declared volume:
# writable, seeded from the image, kept for the session. npm's global prefix
# lives there for the same reason.
RUN useradd --create-home --uid 1000 --shell /bin/bash capsem
USER capsem
WORKDIR /workspace
VOLUME ["/home/capsem"]
ENV PATH=/opt/capsem-debug/python/bin:/home/capsem/.npm-global/bin:/home/capsem/.local/bin:/usr/local/bin:/usr/bin:/bin \
    NPM_CONFIG_PREFIX=/home/capsem/.npm-global \
    DISABLE_AUTOUPDATER=1
RUN set -eux; \
    codex --version; gemini --version; agy --version; pytest --version; \
    python3 -c 'import anthropic, litellm, ollama, openai'; \
    iperf3 --version; redis-benchmark --version; command -v wrk dig nc socat ping
# A session's workload is the image's command, and a session ends with it. The
# suites drive this image through exec, so its workload only has to stay up.
CMD ["sleep", "infinity"]
