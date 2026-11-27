#!/bin/sh
# Drop a schema copy made by copy_schema.sh, ending its database sessions first: the Forms runtimes' sessions can
# outlive their container for a while. A schema that does not exist is fine. Usage: drop_schema.sh SCHEMA
set -eu
schema=$1
case "$schema" in SUMMIT_?*) ;; *) echo "only schema copies (SUMMIT_...) are dropped" >&2; exit 1 ;; esac
docker exec -i forms-oracle-db bash -c 'sqlplus -s / as sysdba' <<SQL
alter session set container=FREEPDB1;
begin
  for attempt in 1 .. 30 loop
    for s in (select sid, serial# serial from v\$session where username = '$schema') loop
      execute immediate 'alter system kill session ''' || s.sid || ',' || s.serial || ''' immediate';
    end loop;
    begin
      execute immediate 'drop user $schema cascade';
      return;
    exception when others then
      if sqlcode = -1918 then return; end if;  -- no such user: setup never made it
      if attempt = 30 then raise; end if;
      dbms_session.sleep(2);
    end;
  end loop;
end;
/
SQL
