# syntax=docker/dockerfile:1.7
# Oracle Forms 14.1.2 install image. Build locally from the OTN installers; never push it.
#   docker buildx build --load -t summit-forms:14.1.2 \
#     --build-context installers=<dir with the Oracle installers> <this directory>
FROM oraclelinux:8

RUN dnf -y install oracle-epel-release-el8 && \
    dnf -y install binutils gcc gcc-c++ glibc glibc-devel libaio libaio-devel libgcc libstdc++ \
        libstdc++-devel libnsl libnsl2 sysstat motif motif-devel libXtst libXrender libXi make \
        ksh unzip tar procps-ng hostname which fontconfig dejavu-sans-fonts dejavu-sans-mono-fonts \
        xorg-x11-server-Xvfb xdotool && \
    dnf clean all

RUN groupadd -g 1000 oinstall && useradd -u 1000 -g oinstall -m oracle && \
    mkdir -p /u01/oracle /u01/jdk /u01/inventory && chown -R oracle:oinstall /u01

ADD --chown=oracle:oinstall https://download.oracle.com/java/17/archive/jdk-17.0.12_linux-x64_bin.tar.gz /tmp/jdk.tgz
RUN tar -xzf /tmp/jdk.tgz -C /u01/jdk --strip-components=1 && rm /tmp/jdk.tgz && chown -R oracle:oinstall /u01/jdk

COPY --chown=oracle:oinstall install/ /u01/install/
USER oracle
ENV JAVA_HOME=/u01/jdk ORACLE_HOME=/u01/oracle PATH=/u01/jdk/bin:/u01/oracle/bin:$PATH

RUN --mount=type=bind,from=installers,source=x,target=/inst \
    java -jar /inst/fmw_14.1.2.0.0_infrastructure.jar -silent \
      -responseFile /u01/install/infrastructure.rsp -invPtrLoc /u01/install/oraInst.loc \
      -ignoreSysPrereqs || (cat /tmp/OraInstall*/install*.log 2>/dev/null | tail -50; exit 1)

RUN --mount=type=bind,from=installers,source=x,target=/inst \
    /inst/fmw_14.1.2.0.0_fr_linux64.bin -silent -responseFile /u01/install/forms.rsp \
      -invPtrLoc /u01/install/oraInst.loc -ignoreSysPrereqs \
    || (cat /tmp/OraInstall*/install*.log 2>/dev/null | tail -80; exit 1)

WORKDIR /u01
