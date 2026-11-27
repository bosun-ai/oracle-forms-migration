-- A private copy of the Summit application schema (every S_ table of SUMMIT with its rows, named constraints,
-- indexes and triggers, and SUMMIT's sequences), with the order tables and order sequence then reset to the
-- baseline snapshot. Used for E5 trials and grades and for the E3 mutants, so each runs on the legacy's own rules.
-- The copy is built from the data dictionary: this database image has no XML DB, so Data Pump and DBMS_METADATA
-- fail (ORA-00600 "unable to load XDB library"). LONG columns (S_IMAGE.IMAGE, S_WAREHOUSE.ADDRESS) become LOBs.
-- Run as SYSDBA in FREEPDB1 through copy_schema.sh, which also places reset.sql in /tmp:
--   sqlplus / as sysdba @copy_schema.sql SCHEMA PASSWORD
whenever sqlerror exit failure
set serveroutput on verify off feedback off
declare
  s constant varchar2(128) := upper('&1');
  password constant varchar2(128) := '&2';
  existing number;
  missing number;
  procedure run(statement varchar2) is begin execute immediate statement; end;
  function key_columns(owner_name varchar2, key_name varchar2) return varchar2 is
    names varchar2(4000);
  begin
    select listagg('"' || column_name || '"', ', ') within group (order by position) into names
      from dba_cons_columns where owner = owner_name and constraint_name = key_name;
    return names;
  end;
  function key_table(owner_name varchar2, key_name varchar2) return varchar2 is
    name varchar2(128);
  begin
    select table_name into name from dba_constraints where owner = owner_name and constraint_name = key_name;
    return name;
  end;
begin
  if not regexp_like(s, '^SUMMIT_[A-Z0-9_]+$') or not regexp_like(password, '^[A-Za-z0-9]{8,64}$') then
    raise_application_error(-20000, 'schema must be SUMMIT_<name>, password alphanumeric');
  end if;
  select count(*) into existing from dba_users where username = s;
  if existing > 0 then run('drop user ' || s || ' cascade'); end if;
  run('create user ' || s || ' identified by "' || password || '" default tablespace users quota unlimited on users');
  run('grant connect, resource, create view, create sequence, create trigger, create procedure, create synonym to ' || s);

  -- Tables and rows. CTAS also copies NOT NULL as unnamed constraints; those are dropped and SUMMIT's own added below.
  for t in (select table_name from dba_tables where owner = 'SUMMIT' and table_name like 'S\_%' escape '\') loop
    declare
      select_list varchar2(4000);
    begin
      select listagg(case when data_type like 'LONG%' then 'to_lob("' || column_name || '") "' || column_name || '"'
                          else '"' || column_name || '"' end, ', ') within group (order by column_id)
        into select_list from dba_tab_columns where owner = 'SUMMIT' and table_name = t.table_name;
      run('create table ' || s || '."' || t.table_name || '" as select ' || select_list || ' from summit."' || t.table_name || '"');
    end;
    for c in (select constraint_name from dba_constraints where owner = s and table_name = t.table_name) loop
      run('alter table ' || s || '."' || t.table_name || '" drop constraint "' || c.constraint_name || '"');
    end loop;
  end loop;

  -- SUMMIT's constraints on those tables, by name: NOT NULL and checks, keys, then foreign keys.
  for c in (select table_name, constraint_name, constraint_type, search_condition_vc condition, r_owner,
                   r_constraint_name, delete_rule, status, validated
              from dba_constraints where owner = 'SUMMIT' and table_name like 'S\_%' escape '\'
             order by decode(constraint_type, 'C', 1, 'P', 2, 'U', 3, 4), table_name, constraint_name) loop
    declare
      name varchar2(200) := case when c.constraint_name not like 'SYS\_%' escape '\' then 'constraint "' || c.constraint_name || '" ' end;
      not_null varchar2(200) := regexp_substr(c.condition, '^"([A-Z0-9_$#]+)" IS NOT NULL$', 1, 1, null, 1);
      clause varchar2(4000);
    begin
      if c.constraint_type = 'C' and not_null is not null then
        clause := 'modify ("' || not_null || '" ' || name || 'not null)';
      else
        clause := 'add ' || name || case c.constraint_type
          when 'C' then 'check (' || c.condition || ')'
          when 'P' then 'primary key (' || key_columns('SUMMIT', c.constraint_name) || ')'
          when 'U' then 'unique (' || key_columns('SUMMIT', c.constraint_name) || ')'
          when 'R' then 'foreign key (' || key_columns('SUMMIT', c.constraint_name) || ') references ' || s || '."'
            || key_table(c.r_owner, c.r_constraint_name)
            || '" (' || key_columns(c.r_owner, c.r_constraint_name) || ')'
            || case c.delete_rule when 'CASCADE' then ' on delete cascade' when 'SET NULL' then ' on delete set null' end
          end;
      end if;
      run('alter table ' || s || '."' || c.table_name || '" ' || clause
          || case when c.status = 'DISABLED' then ' disable' when c.validated = 'NOT VALIDATED' then ' enable novalidate' end);
    end;
  end loop;

  -- Indexes that do not back a key, triggers and sequences.
  for i in (select table_name, index_name, uniqueness from dba_indexes where owner = 'SUMMIT'
              and table_name like 'S\_%' escape '\' and index_type = 'NORMAL'
              and index_name not in (select index_name from dba_constraints where owner = 'SUMMIT' and index_name is not null)) loop
    declare
      names varchar2(4000);
    begin
      select listagg('"' || column_name || '"', ', ') within group (order by column_position) into names
        from dba_ind_columns where index_owner = 'SUMMIT' and index_name = i.index_name;
      run('create ' || case i.uniqueness when 'UNIQUE' then 'unique ' end || 'index ' || s || '."' || i.index_name
          || '" on ' || s || '."' || i.table_name || '" (' || names || ')');
    end;
  end loop;
  -- Trigger descriptions name their table without a schema.
  run('alter session set current_schema = ' || s);
  for g in (select trigger_name, description, trigger_body from dba_triggers
             where owner = 'SUMMIT' and table_name like 'S\_%' escape '\') loop
    run('create or replace trigger ' || replace(g.description, '"SUMMIT".', '"' || s || '".') || g.trigger_body);
  end loop;
  run('alter session set current_schema = SYS');
  for q in (select * from dba_sequences where sequence_owner = 'SUMMIT') loop
    run('create sequence ' || s || '."' || q.sequence_name || '" minvalue ' || q.min_value || ' maxvalue ' || q.max_value
        || ' increment by ' || q.increment_by || ' start with ' || q.last_number
        || case when q.cache_size > 0 then ' cache ' || q.cache_size else ' nocache' end
        || case q.cycle_flag when 'Y' then ' cycle' end || case q.order_flag when 'Y' then ' order' end);
  end loop;
  -- The form and its menu call SUMMIT's stored function GET_PRODUCT_IMAGE (when product images are shown).
  run('grant execute on summit.get_product_image to ' || s);
  run('create or replace synonym ' || s || '.get_product_image for summit.get_product_image');

  -- The copy must carry every named constraint, index and trigger of SUMMIT's S_ tables.
  select count(*) into missing from (
    (select object_type, name from (
       select 'CONSTRAINT' object_type, constraint_name name, owner from dba_constraints
        where table_name like 'S\_%' escape '\' and constraint_name not like 'SYS\_%' escape '\'
       union all select 'INDEX', index_name, owner from dba_indexes
        where table_name like 'S\_%' escape '\' and index_name not like 'SYS\_%' escape '\'
       union all select 'TRIGGER', trigger_name, owner from dba_triggers where table_name like 'S\_%' escape '\')
     where owner = 'SUMMIT'
     minus
     select object_type, name from (
       select 'CONSTRAINT' object_type, constraint_name name, owner from dba_constraints
        where constraint_name not like 'SYS\_%' escape '\'
       union all select 'INDEX', index_name, owner from dba_indexes
       union all select 'TRIGGER', trigger_name, owner from dba_triggers)
     where owner = s));
  if missing > 0 then raise_application_error(-20002, s || ' lacks ' || missing || ' of SUMMIT''s constraints, indexes or triggers'); end if;
end;
/
alter session set current_schema = &1;
@/tmp/reset.sql
