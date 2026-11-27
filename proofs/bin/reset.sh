#!/bin/sh
# Before each case: restore the order tables and the order sequence to the baseline snapshot.
# Parity also runs this while the service is up, so it must not touch the server.
set -e
. "$(dirname "$0")/oracle-env.sh" "$1"
leftovers=$({ ps -eo pid=,ppid=,args= | awk '$2 == 1 && /frmsal\.jar/ { print $1 }'
  ps -eo pid=,etimes=,comm= | awk '$3 == "frmweb" && $2 > 600 { print $1 }'; } 2>/dev/null)
if [ -n "$leftovers" ]; then
  kill $leftovers 2>/dev/null || true
  sleep 3  # their locks go once they have ended
fi
$SQLPLUS @/u01/legacy/db/reset.sql > /dev/null
