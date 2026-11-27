#!/bin/sh
set -e
export TARGET_URL="http://127.0.0.1:${PARITY_PORT_HTTP:-8080}"
exec sh /u01/target/bin/driver.sh "$@"
