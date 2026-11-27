# syntax=docker/dockerfile:1.7
# Summit ORDERS on Oracle Forms 14.1.2 with Parity 0.3.0, built on the local install image. The driver is not in the
# image: each container runs the copy in its proof project (container.sh) or the E5 kit.
#   docker buildx build --load -t summit-forms-runtime:3 \
#     --build-context forms-src=<dir with the Summit sources> -f Runtime.Dockerfile <context>
#   The context must hold these scripts as legacy/ (this directory renamed); the COPY lines read legacy/...

# Parity releases need glibc 2.35+; Oracle Linux 8 has 2.28. Ship Parity with its own loader and libraries.
FROM ubuntu:24.04 AS parity
ARG PARITY_VERSION=v0.3.0
RUN apt-get update -qq && apt-get install -y -qq curl xz-utils ca-certificates >/dev/null && \
    curl --proto '=https' --tlsv1.2 -LsSf \
      https://github.com/bosun-ai/parity-releases/releases/download/${PARITY_VERSION}/parity-cli-installer.sh \
      | env PARITY_CLI_UNMANAGED_INSTALL=/usr/local/bin sh && \
    mkdir -p /opt/parity/bin /opt/parity/lib && cp /usr/local/bin/parity /opt/parity/bin/ && \
    cp -L /lib/x86_64-linux-gnu/libc.so.6 /lib/x86_64-linux-gnu/libm.so.6 \
      /lib/x86_64-linux-gnu/libgcc_s.so.1 /lib64/ld-linux-x86-64.so.2 /opt/parity/lib/

FROM summit-forms:14.1.2
USER root
RUN dnf -y install ImageMagick tini python3 && dnf clean all && fc-cache -f
COPY --from=parity /opt/parity /opt/parity
RUN printf '#!/bin/sh\nexec /opt/parity/lib/ld-linux-x86-64.so.2 --library-path /opt/parity/lib /opt/parity/bin/parity "$@"\n' \
      > /usr/local/bin/parity && chmod 755 /usr/local/bin/parity && parity --version

COPY --chown=oracle:oinstall legacy/create_domain.py legacy/configure_summit.sh legacy/compile.sh legacy/setup.sh legacy/warmup.sh /u01/legacy/
COPY --chown=oracle:oinstall legacy/db/ /u01/legacy/db/
COPY --from=forms-src --chown=oracle:oinstall ORDERS.fmb Form_Builder_II.olb calendar.pll m_summit.mmb /u01/legacy/src/
USER oracle
# The admin password of this local-only development domain is random per build and kept nowhere: nothing in the
# harness logs in as the admin. It is a shell variable, not a build argument, so the image history does not record it.
RUN password="$(od -An -N12 -tx1 /dev/urandom | tr -d ' \n')Wl1" \
    && /u01/oracle/oracle_common/common/bin/wlst.sh /u01/legacy/create_domain.py /u01/domains/forms "$password" \
    && test -f /u01/domains/forms/config/config.xml
