-- Snapshot the Summit order tables once, straight after loading init-summit.sql, so every case can start
-- from the same rows. An existing snapshot is kept: a later run must never capture test data as baseline.
whenever sqlerror exit failure
declare
  existing number;
begin
  for t in (select 'S_ORD' n from dual union all select 'S_ITEM' from dual union all select 'S_ITEM_AUDIT' from dual) loop
    select count(*) into existing from user_tables where table_name = t.n || '_BASE';
    if existing = 0 then
      execute immediate 'create table ' || t.n || '_BASE as select * from ' || t.n;
    end if;
  end loop;
end;
/
exit
