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
CMD ["codex"]
