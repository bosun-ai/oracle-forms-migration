#!/bin/sh
# Make SCHEMA a private copy of the Summit schema in the forms-oracle-db container (copy_schema.sql).
# Usage: echo PASSWORD | copy_schema.sh SCHEMA
# The password comes on stdin so that it never appears in a process list or a command log.
set -eu
schema=$1
read -r password
case "$schema" in SUMMIT_*) ;; *) echo "schema must be SUMMIT_<name>" >&2; exit 1 ;; esac
case "$schema$password" in *[!A-Za-z0-9_]*) echo "schema and password must be alphanumeric" >&2; exit 1 ;; esac
here=$(dirname "$0")
docker cp "$here/copy_schema.sql" forms-oracle-db:/tmp/copy_schema.sql
docker cp "$here/reset.sql" forms-oracle-db:/tmp/reset.sql
printf 'alter session set container=FREEPDB1;\n@/tmp/copy_schema.sql %s %s\n' "$schema" "$password" \
  | docker exec -i forms-oracle-db bash -c 'sqlplus -s / as sysdba'
echo "$schema ready"
