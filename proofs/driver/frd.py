"""Read what an Oracle Forms session showed from its Forms Runtime Diagnostics (FRD) log.

FRD writes a "State Delta" after each event: the form's current block and item, query mode, and per block its
current record and the field values and item properties (ENABLED, MANDATORY, ...) whose display changed. A value
stays on screen until a delta changes it, also when the current record changes: a new record whose default equals
what was displayed logs nothing for that field. Replaying the deltas in that way gives the values the form displayed
for each record. MESSAGE built-in calls and Forms' own messages ("Error Message: FRM-...") give the messages the user
saw on the message line; "<NAME> Trigger Fired" lines and SHOW_ALERT calls say which triggers ran and which alerts
opened.
"""
import re

HIDDEN_FIELDS = {"ROWID"}  # Oracle row addresses: storage internals, not behavior.
PROPERTIES = {"ENABLED", "MANDATORY"}  # item properties the user sees: greyed out, required


class FormState:
    def __init__(self):
        self.records = {}  # block -> {record number -> {field -> value}}
        self.current = {}  # block -> current record number
        self.display = {}  # block -> {field -> value on screen for the current record}
        self.properties = {}  # (block, field) -> {property -> TRUE/FALSE}
        self.focus = {"block": None, "field": None}
        self.query_mode = False
        self.messages = []
        self.triggers = []  # (trigger, block, item) in firing order; block and item None at form level
        self.alerts = []  # (alert name, message text or None) per SHOW_ALERT call, in order

    def show(self, block, record):
        """The current record changed: it starts with what the screen shows, until deltas change it."""
        self.current[block] = record
        self.records.setdefault(block, {})[record] = dict(self.display.get(block, {}))

    def set(self, block, field, value):
        self.display.setdefault(block, {})[field] = value
        self.records.setdefault(block, {}).setdefault(self.current[block], {})[field] = value

    def block(self, name):
        return [dict(sorted(fields.items())) for _, fields in sorted(self.records.get(name, {}).items())]

    def current_record(self, block):
        return dict(self.records.get(block, {}).get(self.current.get(block, 1), {}))

    def property(self, block, field, name):
        return self.properties.get((block, field), {}).get(name)


def _value(raw):
    """FRD quotes values that contain spaces, and writes an empty value as ""."""
    raw = raw.strip()
    if len(raw) >= 2 and raw[0] == raw[-1] == '"':
        return raw[1:-1]
    return raw


def _argument(lines, index):
    """The value of the built-in argument on line `index`, or None."""
    match = re.search(r"Value: (.*)$", lines[index]) if index < len(lines) else None
    return match.group(1).strip() if match else None


def parse(text):
    """Replay an FRD log into the final displayed state, the messages shown, the triggers fired and alerts shown."""
    state = FormState()
    block = field = None
    alert_names, alert_texts = {}, {}  # alert id -> name (FIND_ALERT), id -> message (SET_ALERT_PROPERTY)
    lines = text.splitlines()
    for index, line in enumerate(lines):
        stripped = line.strip()
        if line.startswith("Executing MESSAGE Built-in"):
            message = _argument(lines, index + 1)
            if message is not None:
                state.messages.append(message)
        elif line.startswith("Error Message: "):
            state.messages.append(line[len("Error Message: "):].strip())
        elif line.endswith(" Trigger Fired:"):
            context = {}
            for detail in lines[index + 1:index + 4]:
                key, _, value = detail.partition(": ")
                if key in ("Block", "Item"):
                    context[key] = value.strip()
            state.triggers.append((line[:-len(" Trigger Fired:")], context.get("Block"), context.get("Item")))
        elif line.startswith("Executing FIND_ALERT Built-in"):
            alert_names[_argument(lines, index + 2)] = _argument(lines, index + 1).upper()
        elif line.startswith("Executing SET_ALERT_PROPERTY Built-in") and _argument(lines, index + 2) == "1700":
            alert_texts[_argument(lines, index + 1)] = _argument(lines, index + 3)  # 1700: ALERT_MESSAGE_TEXT
        elif line.startswith("Executing SHOW_ALERT Built-in"):
            alert = _argument(lines, index + 1)
            state.alerts.append((alert_names.get(alert, alert.upper()), alert_texts.get(alert)))
        elif line.startswith("  ") and not line.startswith("   ") and not stripped.startswith("BLOCK "):
            key, _, value = stripped.partition(" ")
            value = _value(value)
            if key == "CURBLOCK":
                state.focus = {"block": value or None, "field": None}
            elif key == "CURFIELD":
                state.focus["field"] = value or None
            elif key == "ENTQUERY":
                state.query_mode = value == "TRUE"
        elif line.startswith("  BLOCK "):
            block, field = stripped.split(" ", 1)[1], None
            if block not in state.current:
                state.show(block, 1)
        elif block and line.startswith("    CURREC "):
            state.show(block, int(stripped.split()[1]))
        elif block and line.startswith("    RECCOUNT "):
            count = int(stripped.split()[1])
            records = state.records.setdefault(block, {})
            for number in [n for n in records if n > count]:
                del records[number]
        elif block and line.startswith("    FIELD "):
            field = stripped.split(" ", 1)[1]
        elif block and field and line.startswith("      "):
            key, _, value = stripped.partition(" ")
            value = _value(value)
            if key == "VALUE" and field not in HIDDEN_FIELDS and value != "[IGNORE]":
                state.set(block, field, value)
            elif key in PROPERTIES and value != "[IGNORE]":
                state.properties.setdefault((block, field), {})[key] = value
        elif line and not line.startswith(" "):
            block = field = None
    return state
