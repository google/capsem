# check=skip=InvalidDefaultArgInFrom
# Claude Desktop on the Capsem base, served as one application over Xpra's
# websocket protocol. BASE is the base image by digest. arm64 only:
# install-gui.sh refuses any other architecture, and the publication workflow
# does not build this image for amd64.
ARG BASE
FROM ${BASE}
ARG TARGETARCH
USER root
COPY install-gui.sh /tmp/install-gui.sh
RUN sh /tmp/install-gui.sh "$TARGETARCH" && rm /tmp/install-gui.sh
COPY --chmod=0555 entrypoint.sh /usr/local/bin/capsem-claude-desktop
USER capsem

# The GUI surface. Xpra binds 127.0.0.1:14500 inside the container -- 14500 is
# Xpra's own default port and the one the v0.7 spike proved -- and the runtime
# reaches it only through the gateway's authenticated preview route.
# Claude Desktop is Electron: its main, GPU and renderer processes plus Xvfb
# and Xpra take ~700 MiB at the sign-in screen, and 2 GiB leaves room for a
# working session.
#
# What the runtime must give it (measured under Docker on arm64): /dev/shm of at least 256 MiB -- at Docker's 64 MiB Claude's
# window stays blank -- and, for Chromium's user-namespace sandbox under
# no_new_privs, seccomp must allow clone() with CLONE_NEWUSER|CLONE_NEWPID|
# CLONE_NEWNET and unshare(). Docker's default profile denies both, and
# Claude then aborts at startup.
LABEL org.capsem.contract="1" \
      org.capsem.surface="xpra" \
      org.capsem.surface.port="14500" \
      org.capsem.memory.min="2048"

# Claude's application state, its login keyring and the keyring's unlock
# secret live in the home directory; a named session keeps them.
VOLUME /home/capsem
CMD ["/usr/local/bin/capsem-claude-desktop"]
