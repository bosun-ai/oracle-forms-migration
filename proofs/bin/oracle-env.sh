# Environment for the Oracle client tools and the Forms launcher. Source with the subject as $1.
# The container's legacy schema and its password are in /etc/summit-oracle (SUMMIT_SCHEMA=, SUMMIT_PASSWORD=),
# written when the container is set up. A project may point the target at another schema and Forms configuration
# with bin/target.env (E3 mutants): SCHEMA=summit_m1 and FORMS_CONFIG=summit_m1.
export ORACLE_HOME=/u01/oracle
export LD_LIBRARY_PATH=/u01/oracle/lib
export TNS_ADMIN=/u01/legacy/domain-pristine/config/fmwconfig
export JAVA_HOME=/u01/jdk
export PATH=/u01/jdk/bin:/u01/oracle/bin:/usr/local/bin:/usr/bin:/bin
export DISPLAY=:99
. /etc/summit-oracle
SCHEMA=$SUMMIT_SCHEMA
PASSWORD=$SUMMIT_PASSWORD
FORMS_CONFIG=summit
# Both subjects run the same commands here; only the target reads target.env.
BIN=$(dirname "$0")
if [ "${1:-}" = target ] && [ -f "$BIN/target.env" ]; then
  . "$BIN/target.env"
fi
export FORMS_URL="http://127.0.0.1:7001/forms/frmservlet?config=$FORMS_CONFIG"
SQLPLUS="/u01/oracle/bin/sqlplus -s -L $SCHEMA/$PASSWORD@SUMMIT"
