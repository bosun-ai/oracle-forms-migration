#!/bin/sh
# One-time setup of the Summit ORDERS Forms server in a fresh container, then stop it.
# Afterwards `proof/bin/serve.sh` runs the server (Parity starts it for each audited run).
# Usage: setup.sh DB_HOST DB_SERVICE DB_USER DB_PASSWORD
set -eu
host=$1 service=$2 user=$3 password=$4
domain=/u01/domains/forms app=/u01/apps/summit
config=$domain/config/fmwconfig/servers/AdminServer/applications/formsapp_14.1.2/config
export ORACLE_HOME=/u01/oracle LD_LIBRARY_PATH=/u01/oracle/lib TNS_ADMIN=$domain/config/fmwconfig

(cd "$domain" && nohup bin/startWebLogic.sh > /tmp/admin.log 2>&1 &)
until grep -q -E 'RUNNING mode|FAILED' /tmp/admin.log; do sleep 3; done
grep -q 'RUNNING mode' /tmp/admin.log
# The Forms application writes its configuration on first deployment.
until [ -f "$config/default.env" ]; do sleep 2; done

grep -q '^\[summit\]' "$config/formsweb.cfg" || \
  sh /u01/legacy/configure_summit.sh "$domain" "$app" "$host" "$service" "$user" "$password"
sh /u01/legacy/compile.sh /u01/legacy/src "$app" "$user/$password@SUMMIT" > /tmp/compile.log 2>&1
# Known, accepted messages: FRM-32093 (product image layout warning) and FRM-18108 (the CALENDAR object
# group subclassed from Form_Builder_II.olb does not load; ORDERS keeps its own copy and calendar dialogs
# are out of scope). Anything else fails the setup.
unexpected=$(grep -E 'Compilation errors|PL/SQL ERROR|FRM-[0-9]+' /tmp/compile.log | grep -v -E 'FRM-32093|FRM-18108' || true)
if [ -n "$unexpected" ] || [ ! -s "$app/ORDERS.fmx" ]; then
  echo "$unexpected" >&2
  exit 1
fi

"$ORACLE_HOME"/bin/sqlplus -s -L "$user/$password@SUMMIT" @/u01/legacy/db/baseline.sql > /tmp/baseline.log
curl -fsS -o /u01/legacy/frmsal.jar http://localhost:7001/forms/java/frmsal.jar

# The domain was written at image build time, so stopWebLogic.sh targets the build host's name.
# SIGTERM runs WebLogic's own shutdown.
pkill -TERM -f 'weblogic[.]Server'
while pgrep -f 'weblogic[.]Server' > /dev/null; do sleep 2; done
# WebLogic writes all over its domain at runtime (logs, caches, embedded LDAP and Derby, Enterprise Manager
# files, lock files, even its .ldift seeds). None of that is order-entry behaviour. Keep a pristine copy of
# the configured domain without caches and logs; each run starts the server from a fresh copy in /tmp,
# which audit treats as temporary activity. The domain keeps its path through a link.
server=servers/AdminServer
rm -rf /u01/legacy/domain-pristine /u01/legacy/domain-nodemanager
cp -a "$domain" /u01/legacy/domain-nodemanager
# Node Manager runs outside the audited subject from its own copy; its scripts name the domain path.
grep -rl "$domain" /u01/legacy/domain-nodemanager/bin /u01/legacy/domain-nodemanager/nodemanager \
  | xargs sed -i "s#$domain#/u01/legacy/domain-nodemanager#g"
(cd "$domain" && rm -rf "$server/tmp" "$server/cache" "$server/logs" "$server/adr" "$server/sysman" \
  nodemanager/*.log nodemanager/*.lck nodemanager/*.id derby.log derbyShutdown.log common/db/derby.log edit.lok)
mv "$domain" /u01/legacy/domain-pristine
ln -s /tmp/domain/forms "$domain"
echo "summit forms set up"
