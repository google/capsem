# check=skip=InvalidDefaultArgInFrom
# Standalone Antigravity IDE, not the separate Antigravity App/hub.
# The named gui context is images/gui, shared with Claude Desktop.
ARG BASE
# This is a required local BuildKit context, not a registry image/tag.
# hadolint ignore=DL3006
FROM gui AS gui-files
FROM ${BASE}
ARG TARGETARCH
USER root
COPY --from=gui-files install-xpra.sh /tmp/install-xpra.sh
RUN sh /tmp/install-xpra.sh "$TARGETARCH" && rm /tmp/install-xpra.sh
COPY install-ide.sh /tmp/install-ide.sh
RUN sh /tmp/install-ide.sh "$TARGETARCH" && rm /tmp/install-ide.sh
COPY --from=gui-files --chmod=0555 entrypoint.sh /usr/local/bin/capsem-antigravity-desktop
USER capsem
ENV CAPSEM_GUI_APP=/opt/antigravity-ide/antigravity-ide
LABEL org.capsem.contract="1" \
      org.capsem.surface="xpra" \
      org.capsem.surface.port="14500" \
      org.capsem.memory.min="2048"
VOLUME /home/capsem
CMD ["/usr/local/bin/capsem-antigravity-desktop"]
