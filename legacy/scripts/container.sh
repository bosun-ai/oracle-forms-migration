#!/bin/sh
# Start a Summit Forms container the way the evaluation harness starts audit containers,
# with /u01 as a fresh volume, then set up the Forms server, on its own copy of the Summit schema, and a virtual
# display.
# Usage: container.sh NAME IMAGE RUN_DIR FORMS_SRC SCHEMA [SUITE]
#   RUN_DIR is mounted at /run-dir (read-write). The Parity project (proof/) is copied to RUN_DIR/proof with
#   the Forms sources from FORMS_SRC (ORDERS.fmb, m_summit.mmb, calendar.pll, Form_Builder_II.olb) and the driver.
#   SUITE (a directory with its own Parityfiles and specs, such as hidden/) replaces the project's.
#   SCHEMA (SUMMIT_<name>) is made a fresh copy of the Summit schema with a random password, which only this
#   container receives (/etc/summit-oracle, readable by root and the oinstall group only, so not by a port running
#   as nobody). The caller drops it when done.
set -eu
name=$1 image=$2 run=$3 src=$4 schema=$5 suite=${6:-}
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$run"
chmod 755 "$run"  # the container's users other than root (oracle, the port's nobody) read it, only root writes there
rm -rf "$run/proof"
cp -R "$here/../proof" "$run/proof"
if [ -n "$suite" ]; then
  rm -rf "$run/proof/specs" "$run/proof"/Parityfile*
  cp -R "$suite/." "$run/proof/"
fi
mkdir -p "$run/proof/src" "$run/proof/driver"
# The driver is part of the proof project, so Parity's identity covers it. This copy is the one every session in
# the container runs (warm-up, audit, E1 repeats).
cp "$here/../driver/"*.py "$run/proof/driver/"
for file in ORDERS.fmb m_summit.mmb calendar.pll Form_Builder_II.olb; do cp "$src/$file" "$run/proof/src/"; done
# -v: the container's anonymous volumes hold a copy of the 7 GB image each.
docker rm -f -v "$name" > /dev/null 2>&1 || true
# Resolve the database from /etc/hosts: a lookup through Docker's DNS proxy cannot be paired by audit.
db_ip=$(docker inspect -f '{{(index .NetworkSettings.Networks "forms-net").IPAddress}}' forms-oracle-db)
docker run -d --name "$name" --hostname "${FORMS_HOSTNAME:-$name}" --network forms-net --add-host "oracle-db:$db_ip" \
  --cgroupns private --ulimit memlock=-1:-1 \
  --security-opt apparmor=unconfined --security-opt seccomp=unconfined \
  --tmpfs /tmp:rw,exec,mode=1777 \
  --cap-add SYS_ADMIN --cap-add SYS_PTRACE --cap-add NET_ADMIN --cap-add BPF --cap-add PERFMON \
  -v /usr -v /etc -v /var -v /root -v /opt -v /u01 \
  -v "$(cd "$run" && pwd)":/run-dir \
  "$image" /usr/bin/tini -- sleep infinity > /dev/null
docker exec -u root "$name" sh -c 'set -e; mkdir -p /tmp/forms-cgroup /sys/kernel/tracing /sys/fs/bpf
  mount -t cgroup2 none /tmp/forms-cgroup; mount --bind /tmp/forms-cgroup /sys/fs/cgroup
  mount -t tracefs tracefs /sys/kernel/tracing; mount -t bpf bpffs /sys/fs/bpf'
# The legacy scripts of this checkout, not the image's copies of them.
tar -C "$here" -cf - configure_summit.sh setup.sh warmup.sh compile.sh db \
  | docker exec -i -u root "$name" sh -c 'tar -C /u01/legacy -xf - && chown -R oracle:oinstall /u01/legacy'
password=$(od -An -N12 -tx1 /dev/urandom | tr -d ' \n')
echo "$password" | sh "$here/db/copy_schema.sh" "$schema"
printf 'SUMMIT_SCHEMA=%s\nSUMMIT_PASSWORD=%s\n' "$schema" "$password" \
  | docker exec -i -u root "$name" sh -c 'umask 027; cat > /etc/summit-oracle; chgrp oinstall /etc/summit-oracle'
docker exec "$name" sh -c '. /etc/summit-oracle; sh /u01/legacy/setup.sh oracle-db FREEPDB1 "$SUMMIT_SCHEMA" "$SUMMIT_PASSWORD"'
# Keep the virtual display up for the whole container lifetime.
docker exec -d -u root "$name" sh -c 'while true; do rm -f /tmp/.X99-lock; Xvfb :99 -screen 0 1280x1024x24 -nolisten tcp; sleep 1; done'
# The examiner's screen probe for the Forms dialogs and windows FRD does not log (driver/screen.py), outside the
# audited subject like the display it reads.
docker exec -d -u root "$name" sh -c 'cd /run-dir/proof/driver && exec python3 screen.py serve > /tmp/forms-screen.log 2>&1'
# WebLogic polls the Node Manager of the machine that hosts the forms1 component. Run it outside the
# audited subject, like the database.
docker exec -d "$name" sh -c 'cd /u01/legacy/domain-nodemanager && exec bin/startNodeManager.sh > /tmp/nodemanager.log 2>&1'
# One-time caches before any measurement (see warmup.sh).
docker exec -u root "$name" sh /u01/legacy/warmup.sh /run-dir/proof
# The Forms server runs outside the audited subject, like the database (see proof/Parityfile).
docker exec -d -u root "$name" sh -c 'exec sh /run-dir/proof/bin/serve.sh > /tmp/forms-server.log 2>&1'
docker exec "$name" sh -c 'tries=0
  until curl -fs -o /dev/null http://127.0.0.1:7001/forms/frmservlet; do
    tries=$((tries + 1))
    [ "$tries" -le 450 ] || { echo "the Forms server did not answer within 15 minutes:"; tail -50 /tmp/forms-server.log; exit 1; }
    sleep 2
  done'
