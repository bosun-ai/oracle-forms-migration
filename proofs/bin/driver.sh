#!/bin/sh
# The Parity driver: works the ORDERS form through the Forms client.
. "$(dirname "$0")/oracle-env.sh" "$1"
export PYTHONDONTWRITEBYTECODE=1
# The driver names the subject it serves in its hello; Parity checks it.
export PARITY_SUBJECT="$1"
exec python3 "$(dirname "$0")/../driver/forms_legacy.py"
