# A development session without an agent: the Capsem base plus a compiler
# toolchain and everyday tools. BASE is the base image by digest.
ARG BASE
FROM ${BASE}
USER root
RUN set -eux; \
    apt-get -o Acquire::Check-Valid-Until=false update; \
    apt-get install -y --no-install-recommends \
        build-essential diffutils file gh lsof make man-db pkg-config \
        psmisc strace tmux unzip vim-tiny; \
    ln -sf vim.tiny /usr/bin/vim; \
    rm -rf /var/lib/apt/lists/*
USER capsem
