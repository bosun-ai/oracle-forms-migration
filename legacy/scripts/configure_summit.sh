#!/bin/sh
# Register the Summit ORDERS form in a freshly created Forms domain.
# Usage: configure_summit.sh DOMAIN_HOME APP_DIR DB_HOST DB_SERVICE DB_USER DB_PASSWORD
set -eu
domain=$1 app=$2 host=$3 service=$4 user=$5 password=$6
config=$domain/config/fmwconfig/servers/AdminServer/applications/formsapp_14.1.2/config

# The development template writes the forms1 instance to components/FORMS/forms1, but the servlet looks
# in components/FORMS/instances/forms1.
mkdir -p "$domain/config/fmwconfig/components/FORMS/instances"
ln -sfn ../forms1 "$domain/config/fmwconfig/components/FORMS/instances/forms1"

# The Forms compiler and runtime cannot use EZConnect strings; they need a TNS alias.
cat > "$domain/config/fmwconfig/tnsnames.ora" <<EOF
SUMMIT =
  (DESCRIPTION = (ADDRESS = (PROTOCOL = TCP)(HOST = $host)(PORT = 1521))
    (CONNECT_DATA = (SERVICE_NAME = $service)))
EOF

mkdir -p /tmp/forms-trace
sed "s#^FORMS_PATH=.*#FORMS_PATH=$app:/u01/oracle/forms#" "$config/default.env" > "$config/summit.env"
echo "FORMS_TRACE_DIR=/tmp/forms-trace" >> "$config/summit.env"

# summit_b: the same form for a second user at the same time, tracing to its own FRD log.
for section in summit summit_b; do
  log=/tmp/forms-trace/frd.log
  [ "$section" = summit ] || log=/tmp/forms-trace/frd-b.log
  cat >> "$config/formsweb.cfg" <<EOF

[$section]
form=ORDERS
userid=$user/$password@SUMMIT
envFile=summit.env
workingDirectory=$app
otherparams=record=collect log=$log term=/u01/oracle/forms/templates/resource/US/fmrweb.res
EOF
done
