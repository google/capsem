# Claude Code on the Capsem base. BASE is the base image by digest.
ARG BASE
FROM ${BASE}
ARG TARGETARCH
ARG CLAUDE_VERSION=2.1.229
USER root
RUN set -eu; \
    case "$TARGETARCH" in \
        arm64) platform=linux-arm64 sha256=ba53130acbda3ddb00b3dd5641f11733867f5d837a258b7e6ccef4927cc1a509 ;; \
        amd64) platform=linux-x64 sha256=200338139a3df04a9ad22233837d1fb53fb6dffa21cd82e47559bfaa115acc1b ;; \
        *) echo "unsupported architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    curl -fsSL "https://downloads.claude.ai/claude-code-releases/$CLAUDE_VERSION/$platform/claude" -o /usr/local/bin/claude; \
    echo "$sha256  /usr/local/bin/claude" | sha256sum -c -; \
    chmod 555 /usr/local/bin/claude
USER capsem
ENV DISABLE_AUTOUPDATER=1
RUN claude --version | grep -F "$CLAUDE_VERSION"
# Capsem's MCP tools, at the name the session's proxy answers on the host. A
# container cannot reach the in-guest vsock relay; plain HTTP is enough
# because the connection ends at the VM's own proxy, never on a network.
RUN f=/home/capsem/.claude.json; \
    [ -f "$f" ] || echo '{}' > "$f"; \
    jq '.mcpServers.capsem = {type: "http", url: "http://mcp.capsem.internal/mcp"}' "$f" > "$f.new"; \
    mv "$f.new" "$f"
CMD ["claude"]
