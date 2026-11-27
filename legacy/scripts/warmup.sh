#!/bin/sh
# Run one Forms session outside audit, as the audited subject runs (root, same scripts), so one-time caches
# (the launcher's jar cache, fontconfig caches) exist before any measurement.
# Usage: warmup.sh PROOF_DIR   (a proof project with bin/ and driver/; its driver is the one that runs)
set -eu
proof=$1
cd "$proof"
sh bin/reset.sh
nohup sh bin/serve.sh > /tmp/warmup-serve.log 2>&1 &
server=$!
tries=0
until curl -fs -o /dev/null http://127.0.0.1:7001/forms/frmservlet; do
  tries=$((tries + 1))
  [ "$tries" -le 450 ] || { echo "the Forms server did not answer within 15 minutes:"; tail -50 /tmp/warmup-serve.log; exit 1; }
  sleep 2
done
. bin/oracle-env.sh
PYTHONDONTWRITEBYTECODE=1 python3 driver/forms_legacy.py --steps '[{"action": "enter_query"},
  {"action": "type", "input": {"item": "order-id", "value": 100}}, {"action": "execute_query"}]' > /tmp/warmup.json
# startWebLogic.sh runs the server JVM as a child; stop the JVM itself.
pkill -TERM -f 'weblogic[.]Server'
tries=0
while pgrep -f 'weblogic[.]Server' > /dev/null; do
  tries=$((tries + 1))
  [ "$tries" -le 150 ] || { echo "the Forms server did not stop within 5 minutes:"; tail -50 /tmp/warmup-serve.log; exit 1; }
  sleep 2
done
wait "$server" || true
python3 -c 'import json, sys; sys.exit(json.load(open("/tmp/warmup.json"))[-1].get("order", {}).get("order-id") != 100)'
echo "warm-up session opened order 100"
