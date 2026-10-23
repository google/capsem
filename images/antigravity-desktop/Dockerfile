# check=skip=InvalidDefaultArgInFrom
# Standalone Antigravity IDE, not the separate Antigravity App/hub.
# Xpra and the graphical launcher are inherited from the Capsem base.
ARG BASE
FROM ${BASE}
ARG TARGETARCH
USER root
COPY install-ide.sh /tmp/install-ide.sh
RUN sh /tmp/install-ide.sh "$TARGETARCH" && rm /tmp/install-ide.sh
# Xpra removes its own decorations; the IDE must also hide its native controls.
COPY --chown=capsem:capsem ["settings.json", "/home/capsem/.config/Antigravity IDE/User/settings.json"]
USER capsem
ENV CAPSEM_GUI_APP=/opt/antigravity-ide/antigravity-ide
LABEL org.capsem.contract="1" \
      org.capsem.surface="xpra" \
      org.capsem.surface.port="14500" \
      org.capsem.memory.min="2048"
VOLUME /home/capsem
CMD ["/usr/local/bin/capsem-gui"]
