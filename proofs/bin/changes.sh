#!/bin/sh
# Print the database changes against the baseline snapshot as JSON.
set -e
. "$(dirname "$0")/oracle-env.sh" "$1"
# A web target (E4/E5) reports its PostgreSQL changes in the same shape.
if [ -n "${TARGET_CHANGES:-}" ]; then
  exec sh "$TARGET_CHANGES"
fi
$SQLPLUS @/u01/legacy/db/changes.sql
