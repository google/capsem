# check=skip=InvalidDefaultArgInFrom
# Codex CLI on the Capsem base. BASE is the base image by digest. npm checks
# the registry's integrity hash for the exact version.
ARG BASE
FROM ${BASE}
ARG CODEX_VERSION=0.147.0
USER root
RUN npm install --global --no-fund --no-audit "@openai/codex@$CODEX_VERSION" \
    && npm cache clean --force
USER capsem
RUN codex --version | grep -F "$CODEX_VERSION"
# Capsem's MCP tools, at the name the session's proxy answers on the host. A
# container cannot reach the in-guest vsock relay; plain HTTP is enough
# because the connection ends at the VM's own proxy, never on a network.
COPY --chown=capsem:capsem config.toml /home/capsem/.codex/config.toml
CMD ["codex"]
