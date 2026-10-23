# check=skip=InvalidDefaultArgInFrom
# Google's Antigravity 2 App, the agent-first desktop application.
# Xpra and the graphical launcher are inherited from the Capsem base.
ARG BASE
FROM ${BASE}
ARG TARGETARCH
USER root
COPY install-app.sh /tmp/install-app.sh
RUN sh /tmp/install-app.sh "$TARGETARCH" && rm /tmp/install-app.sh
USER capsem
ENV CAPSEM_GUI_APP=/opt/antigravity/antigravity
LABEL org.capsem.contract="1" \
      org.capsem.surface="xpra" \
      org.capsem.surface.port="14500" \
      org.capsem.workspace.policy="antigravity-scratch-v1" \
      org.capsem.memory.min="2048"
VOLUME /home/capsem
CMD ["/usr/local/bin/capsem-gui"]
