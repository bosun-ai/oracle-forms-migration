-- Restore the Summit order tables and the order sequence to the baseline snapshot.
-- TRUNCATE, not DELETE: rows must land in the same physical order every time. ORDERS queries S_ORD without
-- an ORDER BY, so the order it shows first depends on where the rows sit.
whenever sqlerror exit failure
set feedback off
-- No trigger may act on restored rows (an E3 mutant adds an insert trigger to S_ORD).
alter table s_ord disable all triggers;
alter table s_item disable all triggers;
alter table s_item disable constraint s_item_ord_id_fk;
truncate table s_item;
truncate table s_ord;
truncate table s_item_audit;
insert into s_ord select * from s_ord_base;
insert into s_item select * from s_item_base;
insert into s_item_audit select * from s_item_audit_base;
commit;
alter table s_item enable constraint s_item_ord_id_fk;
alter table s_item enable all triggers;
alter table s_ord enable all triggers;
drop sequence s_ord_id;
-- As in init-summit.sql line 232.
create sequence s_ord_id minvalue 1 maxvalue 999999999999999999999999999 increment by 1
  start with 50000 cache 50 noorder nocycle nokeep noscale global;
exit
