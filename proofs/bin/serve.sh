#!/bin/sh
# Run the Oracle Forms server (WebLogic admin server with the Forms application) in the foreground, from a
# fresh copy of the configured domain in /tmp (see legacy/setup.sh). Forms does not use the Derby database
# that startWebLogic.sh starts by default.
rm -rf /tmp/domain/forms
mkdir -p /tmp/domain
# Plain copy: preserving timestamps would use utimes, which audit does not support.
cp -R /u01/legacy/domain-pristine /tmp/domain/forms
export DERBY_FLAG=false
cd /u01/domains/forms && exec bin/startWebLogic.sh
