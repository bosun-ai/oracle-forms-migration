#!/bin/sh
# Compile the original Summit ORDERS form, its menu and its calendar library against the Summit schema.
# Usage: compile.sh SRC_DIR OUT_DIR USERID   (USERID uses a TNS alias; Forms tools reject EZConnect)
# frmcmp_batch exits 1 on warnings such as FRM-32093 (layout), so success is judged by the outputs and
# the absence of compilation errors in the log.
set -u
src=$1 out=$2 userid=$3
export ORACLE_HOME=/u01/oracle LD_LIBRARY_PATH=/u01/oracle/lib TERM=vt220
export TNS_ADMIN=/u01/domains/forms/config/fmwconfig FORMS_PATH=$out
mkdir -p "$out"
cp "$src"/ORDERS.fmb "$src"/Form_Builder_II.olb "$src"/calendar.pll "$src"/m_summit.mmb "$out"/
cd "$out" || exit 1
compile() { "$ORACLE_HOME"/bin/frmcmp_batch "$@" compile_all=yes userid="$userid"; }
compile module=calendar.pll module_type=LIBRARY
compile module=m_summit.mmb module_type=MENU
compile module=ORDERS.fmb module_type=FORM
test -s calendar.plx && test -s m_summit.mmx && test -s ORDERS.fmx
