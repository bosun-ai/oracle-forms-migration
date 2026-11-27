-- Print, as one JSON document, how the order tables differ from the baseline snapshot.
-- "before" holds baseline rows that changed or disappeared; "after" holds rows that changed or appeared.
-- A date that is today shows as "today", so runs on different days compare (a new order's date defaults to today).
whenever sqlerror exit failure
set long 10000000 longchunksize 10000000 linesize 32767 pagesize 0 trimspool on heading off feedback off
with
  ord_row as (select id, customer_id, case when trunc(date_ordered) = trunc(sysdate) then 'today' else to_char(date_ordered, 'YYYY-MM-DD') end date_ordered,
                     case when trunc(date_shipped) = trunc(sysdate) then 'today' else to_char(date_shipped, 'YYYY-MM-DD') end date_shipped, sales_rep_id, total, payment_type, order_filled
              from s_ord),
  ord_base as (select id, customer_id, case when trunc(date_ordered) = trunc(sysdate) then 'today' else to_char(date_ordered, 'YYYY-MM-DD') end date_ordered,
                      case when trunc(date_shipped) = trunc(sysdate) then 'today' else to_char(date_shipped, 'YYYY-MM-DD') end date_shipped, sales_rep_id, total, payment_type, order_filled
               from s_ord_base),
  ord_after as (select * from ord_row minus select * from ord_base),
  ord_before as (select * from ord_base minus select * from ord_row),
  item_after as (select ord_id, item_id, product_id, price, quantity, quantity_shipped from s_item
                 minus select ord_id, item_id, product_id, price, quantity, quantity_shipped from s_item_base),
  item_before as (select ord_id, item_id, product_id, price, quantity, quantity_shipped from s_item_base
                  minus select ord_id, item_id, product_id, price, quantity, quantity_shipped from s_item)
select json_object(
  'orders' value json_object(
    'before' value (select json_arrayagg(json_object(* absent on null) order by id returning clob) from ord_before),
    'after' value (select json_arrayagg(json_object(* absent on null) order by id returning clob) from ord_after)
    returning clob),
  'items' value json_object(
    'before' value (select json_arrayagg(json_object(* absent on null) order by ord_id, item_id returning clob) from item_before),
    'after' value (select json_arrayagg(json_object(* absent on null) order by ord_id, item_id returning clob) from item_after)
    returning clob),
  'audit_rows_added' value ((select count(*) from s_item_audit) - (select count(*) from s_item_audit_base))
  returning clob)
from dual;
exit
