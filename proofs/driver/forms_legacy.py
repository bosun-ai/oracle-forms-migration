"""Parity driver for the Summit ORDERS form on Oracle Forms.

A case is one or two user sessions on the real form (session "A", and "B" for a second user at the same time). Each
invoke is one user step in a session, started on its first step: the Forms Standalone Launcher under Xvfb, worked with
the mouse and keyboard (xdotool), the way a user works it. The reply is what that session showed after the step,
read from its Forms Runtime Diagnostics (FRD) log, in the canonical observation both drivers report (see
e5/task.md, "What the examiner observes"). At the end of the case every session leaves the form as a user would
(Exit, confirm, do not save).

The header block S_ORD answers most navigation keys with "Function Not Allowed" (its KEY-OTHERS trigger), so items
are reached by clicking, as users do.

Protocol: Parity persistent driver, JSON lines on stdin/stdout. CLI: forms_legacy.py ACTION 'JSON' (one step) or
forms_legacy.py --steps '[{"action": ..., "input": {...}}, ...]' (one case), printing the observations.
"""
import json
import os
import re
import stat
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import frd  # noqa: E402
import screen as probe  # noqa: E402

LAUNCHER = os.environ.get("FORMS_LAUNCHER", "/u01/legacy/frmsal.jar")
URL = os.environ.get("FORMS_URL", "http://127.0.0.1:7001/forms/frmservlet?config=summit")
TRACE_DIR = os.path.dirname(os.environ.get("FORMS_FRD_LOG", "/tmp/forms-trace/frd.log"))
# The steps of every case's sessions, kept for the examiner's trigger coverage (temporary space, outside the proof).
# The legacy's screens, step by step: the expected answers of the hidden suite, so readable by the driver's user only
# (a graded port runs as another user in the same container).
STEP_ARCHIVE = os.environ.get("FORMS_STEP_ARCHIVE", "/tmp/forms-steps")
DISPLAY = os.environ.get("DISPLAY", ":99")

# Screen positions of the ORDERS items in the launcher window at its origin (0, 0).
HEADER = {
    "order-id": (120, 112), "date-ordered": (84, 177), "customer-id": (192, 177), "customer-name": (330, 177),
    "rep-id": (192, 224), "rep-name": (330, 224), "date-shipped": (84, 272), "order-filled": (308, 271),
    "immediate": (30, 66), "auto-query": (30, 83), "order-total": (497, 441),
}
PAYMENT = {"CASH": (164, 271), "CREDIT": (230, 271)}
LINE_ROWS = (350, 369, 389, 408)
LINE = {"item-id": 45, "product-id": 88, "description": 210, "price": 330, "quantity": 380, "shipped": 417,
        "item-total": 495}
EXIT_BUTTON = (422, 72)  # the toolbar's Exit
NEUTRAL = (700, 300)  # the empty area right of the form: a click there gives the window focus, nothing else
MENU = {"File": (15, 14), "Sort By": (93, 14)}
MENU_ITEMS = {("File", "Clear"): (25, 61), ("Sort By", "id"): (120, 40), ("Sort By", "customer_id"): (120, 61),
              ("Sort By", "sales_rep_id"): (120, 81)}

# FRD names of what the examiner observes, by the target's data-testid.
HEADER_FIELDS = {"order-id": "ID", "customer-id": "CUSTOMER_ID", "customer-name": "CUSTOMER_NAME",
                 "rep-id": "SALES_REP_ID", "rep-name": "SALES_REP_NAME", "date-ordered": "DATE_ORDERED",
                 "date-shipped": "DATE_SHIPPED", "payment-type": "PAYMENT_TYPE", "order-filled": "ORDER_FILLED"}
LINE_FIELDS = {"item-id": "ITEM_ID", "product-id": "PRODUCT_ID", "description": "DESCRIPTION", "price": "PRICE",
               "quantity": "QUANTITY", "shipped": "QUANTITY_SHIPPED", "item-total": "ITEM_TOTAL"}
CONTROL_FIELDS = {"immediate": "IMMEDIATE", "auto-query": "AUTO_QUERY"}
# What the toolbar checkboxes show when the form opens; FRD logs their values only once they change. Immediate starts
# checked; Auto Query starts unchecked, whatever its InitializeValue says: the running form's first click on it logs
# VALUE Y and finds the relation's autoquery FALSE.
CONTROL_START = {"IMMEDIATE": "Y", "AUTO_QUERY": "N"}
# Values reported for the line with the cursor only (see canonical()).
CURSOR_LINE_ONLY = {"item-id", "description", "item-total"}
# Items whose enabled and required state is observed: the header's and the toolbar's.
ITEMS = dict([(t, ("S_ORD", f)) for t, f in HEADER_FIELDS.items()] + [(t, ("CONTROL", f)) for t, f in CONTROL_FIELDS.items()])

# The form's alerts (ORDERS_fmb.xml), as the user sees them; SET_ALERT_PROPERTY may replace the text.
ALERTS = {"DELETE_ALERT": ("Forms", None, ["Yes", "No"]), "QUESTION_ALERT": ("Question", None, ["Yes", "No"]),
          "PAYMENT_TYPE_ALERT": ("Payment Type", "This customer must pay cash!", ["OK"])}
# Forms' own dialogs, which FRD does not log, told apart by the width of their title bar on screen.
BUILTIN_DIALOGS = {434: ("Forms", "Do you want to save the changes you have made?", ["Yes", "No", "Cancel"]),
                   414: ("Forms", "Could not reserve record (2 tries). Keep trying?", ["Yes", "No"])}
ANSWER_KEYS = {"Yes": "y", "No": "n", "Cancel": "Escape", "OK": "Return"}
MONTHS = ("JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC")


def forms_date(iso):
    """2026-04-08 -> 08-APR-2026, the form's display format."""
    year, month, day = iso.split("-")
    return "%s-%s-%s" % (day, MONTHS[int(month) - 1], year)


def iso_date(shown):
    """08-APR-2026 -> 2026-04-08, today's date -> "today" (runs on different days compare); anything else as shown."""
    match = re.fullmatch(r"(\d{2})-([A-Z]{3})-(\d{4})", shown)
    if not match or match.group(2) not in MONTHS:
        return shown
    iso = "%s-%02d-%s" % (match.group(3), MONTHS.index(match.group(2)) + 1, match.group(1))
    return "today" if iso == time.strftime("%Y-%m-%d", time.gmtime()) else iso


def integer(text):
    """A whole number as a number; anything else (text the user typed) as it was shown."""
    return int(text) if re.fullmatch(r"-?\d+", text) else text


def money(text):
    return "%.2f" % float(text) if re.fullmatch(r"-?\d+(\.\d+)?", text) else text


HEADER_VALUES = {"order-id": integer, "customer-id": integer, "rep-id": integer, "date-ordered": iso_date,
                 "date-shipped": iso_date}
LINE_VALUES = {"item-id": integer, "product-id": integer, "price": money, "quantity": integer, "shipped": integer,
               "item-total": money}


def distinct(messages):
    shown = []
    for message in messages:
        if message not in shown:
            shown.append(message)
    return shown


def canonical(state, messages):
    """What the user saw after a step, in the form both drivers report (e5/task.md).

    FRD logs a line's values whenever they change while it is the current line, but values a trigger sets on another
    line (POST-QUERY's description, the item total formula, the line number PRE-INSERT gives each new line at a
    commit) only once that line becomes current. So line numbers, descriptions and item totals are reported for the
    line with the cursor only; the other line values for every line."""
    current = state.current_record("S_ORD")
    order = {}
    for testid, field in HEADER_FIELDS.items():
        if current.get(field) not in (None, ""):
            order[testid] = HEADER_VALUES.get(testid, str)(current[field])
    where = focus(state)
    cursor_line = where.get("line") if where else None
    lines = []
    for number, record in enumerate(state.block("S_ITEM"), 1):
        line = {testid: LINE_VALUES.get(testid, str)(record[field]) for testid, field in LINE_FIELDS.items()
                if record.get(field) not in (None, "") and (number == cursor_line or testid not in CURSOR_LINE_ONLY)}
        if line:
            lines.append(line)
    controls = state.display.get("CONTROL", {})
    observation = {"order": order, "lines": lines, "focus": where, "mode": "query" if state.query_mode else "normal",
                   "disabled": sorted(t for t, (block, f) in ITEMS.items() if state.property(block, f, "ENABLED") != "TRUE"),
                   "required": sorted(t for t, (block, f) in ITEMS.items() if state.property(block, f, "MANDATORY") == "TRUE"),
                   "options": {t: controls.get(f, CONTROL_START[f]) == "Y" for t, f in CONTROL_FIELDS.items()},
                   "messages": distinct(messages)}
    total = state.display.get("S_ITEM", {}).get("TOTAL")
    if total:
        observation["total"] = money(total)
    return observation


def form_alert(name, text):
    """One of the form's alerts as the user sees it: its title, its text (SET_ALERT_PROPERTY's, else its own) and its
    buttons."""
    title, default, buttons = ALERTS[name]
    return {"title": title, "text": text or default, "buttons": buttons}


def screen(log, step_start, step_end):
    """The observation of a step that ran from `step_start` to `step_end` of a session's FRD log, which continues with
    the driver's Exit: the step's display changes are in the header of that Exit event."""
    exit_key = log.find("\nKEY Exit\n", step_end)
    if exit_key < 0:
        raise RuntimeError("no Exit after the step: its display changes are not in the log")
    header_end = log.rfind("\nEND\n", step_end, exit_key)
    return canonical(frd.parse(log[:header_end]), frd.parse(log[step_start:step_end]).messages)


def focus(state):
    """The item the cursor is in, by test id, with its line number for a line item."""
    block, field = state.focus["block"] or "S_ORD", state.focus["field"]
    names = {"S_ORD": HEADER_FIELDS, "S_ITEM": LINE_FIELDS, "CONTROL": CONTROL_FIELDS}.get(block, {})
    testid = next((t for t, f in names.items() if f == field), None)
    if testid is None:
        return None
    return {"item": testid, "line": state.current.get("S_ITEM", 1)} if block == "S_ITEM" else {"item": testid}


def xdotool(*args):
    subprocess.run(["xdotool"] + [str(a) for a in args], check=True, env=dict(os.environ, DISPLAY=DISPLAY))


class Session:
    """One Forms client from launch to exit."""

    def __init__(self, name):
        self.name = name
        os.makedirs(TRACE_DIR, exist_ok=True)
        self.log_path = os.path.join(TRACE_DIR, "frd.log" if name == "A" else "frd-%s.log" % name.lower())
        # A second user runs the same form in its own configuration, which traces to its own log.
        url = URL if name == "A" else re.sub(r"config=(\w+)", r"config=\1_" + name.lower(), URL)
        if os.path.exists(self.log_path):
            os.remove(self.log_path)
        probe.ask("mark")
        client_log = open(os.path.join(TRACE_DIR, "client-%s.log" % name.lower()), "w")
        # No JVM performance data: a JVM that finds another's stale file probes that process with a signal, which
        # audit records as a signal to a process outside the subject, in some runs and not others.
        self.client = subprocess.Popen(["java", "-XX:-UsePerfData", "-jar", LAUNCHER, "-url", url, "-t", "30000"],
                                       stdout=client_log, stderr=subprocess.STDOUT,
                                       env=dict(os.environ, DISPLAY=DISPLAY))
        client_log.close()
        self.steps = []  # (start, end) offsets of the user's steps in the log, for coverage
        try:
            # Under audit the client JVM starts far slower than natively (about 5 s).
            deadline = time.time() + 280
            while "WHEN-NEW-RECORD-INSTANCE" not in self.text():
                with open(client_log.name, errors="replace") as output:
                    launch_error = "FRM-93" in output.read()
                if launch_error or time.time() > deadline or self.client.poll() is not None:
                    raise RuntimeError("the ORDERS form did not open: " + startup_failure(client_log.name))
                time.sleep(0.2)
            self.settle()
            probe.ask("claim " + name)
            self.runtime = self.runtime_pid()
            self.click(HEADER["order-id"])
        except BaseException:
            # No session to leave: stop the client, whose runtime then ends with its connection.
            self.client.kill()
            self.client.wait()
            raise
        self.step_start = self.unseen_from = len(self.text())
        self.steps.append((0, self.step_start))  # opening the form is the user's first step
        self.dialog_open, self.boxes_seen = None, 0
        self.focus = {"item": "order-id"}

    def text(self):
        try:
            with open(self.log_path, errors="replace") as log:
                return log.read()
        except OSError:
            return ""

    def size(self):
        try:
            return os.path.getsize(self.log_path)
        except OSError:
            return -1

    def settle(self, quiet=0.8, limit=30.0):
        """Wait until Forms stops writing this session's FRD log: the form has finished reacting."""
        deadline = time.time() + limit
        last, since = self.size(), time.time()
        while time.time() < deadline:
            time.sleep(0.1)
            size = self.size()
            if size != last:
                last, since = size, time.time()
            elif size > 0 and time.time() - since >= quiet:
                return
        raise TimeoutError("form did not settle within %ss" % limit)

    def runtime_pid(self):
        for line in self.text().splitlines():
            if line.startswith("Process ID:"):
                return int(line.split(":")[1])
        raise RuntimeError("FRD log names no Forms runtime process")

    def activate(self):
        """Bring this session's window to the front and give it the keyboard."""
        probe.ask("raise " + self.name)
        self.click(NEUTRAL)

    def click(self, position):
        xdotool("mousemove", position[0], position[1], "click", 1)
        self.settle()

    def keys(self, *names):
        xdotool("key", *names)
        self.settle()

    def replace(self, position, value):
        """Click an item, clear what it holds and type the new value, as a user would."""
        xdotool("mousemove", position[0], position[1], "click", 1)
        xdotool("key", "End", *(["BackSpace"] * 16))
        if value not in (None, ""):
            xdotool("type", "--delay", 40, "--", str(value))
        self.settle()

    def dialog(self):
        """The alert or Forms dialog open in this session, or None."""
        width = int(probe.ask("dialog"))
        if not width:
            return None
        log = self.text()
        if log.rfind("Executing SHOW_ALERT") > log.rfind("\nALERT "):  # the form's own alert, still unanswered
            return form_alert(*frd.parse(log).alerts[-1])
        if width in BUILTIN_DIALOGS:
            title, text, buttons = BUILTIN_DIALOGS[width]
            return {"title": title, "text": text, "buttons": buttons}
        # Otherwise a message box: a message the user has not seen yet when the next one comes, held until they answer
        # OK. Boxes show the unseen messages in order.
        messages = frd.parse(log[self.unseen_from:]).messages
        text = messages[self.boxes_seen] if self.boxes_seen < len(messages) else "unknown dialog (%d px)" % width
        return {"title": "Forms", "text": text, "buttons": ["OK"]}

    def observe(self):
        """What the session shows after a step. FRD writes a step's display changes only in the header of the next
        event, so the driver sends one that changes nothing: Exit (F4), then "No" to "Do you really want to leave the
        form?". Exit runs no validation and moves no cursor."""
        self.dialog_open = self.dialog()
        log = self.text()
        if self.dialog_open:
            # The screen behind a dialog is not settled until it is answered; only the dialog is observed.
            self.steps.append((self.step_start, len(log)))
            observation = {"alert": self.dialog_open, "messages": distinct(frd.parse(log[self.step_start:]).messages)}
            self.step_start = len(log)
            return observation
        step_end = len(log)
        self.keys("F4")
        observation = screen(self.text(), self.step_start, step_end)
        self.keys(ANSWER_KEYS["No"])
        self.steps.append((self.step_start, step_end))
        self.step_start = self.unseen_from = len(self.text())
        self.boxes_seen = 0
        self.focus = observation["focus"] or {}
        return observation

    def to_header(self):
        """Header functions (query, new, delete, ...) act on the block with the cursor: go to the header first."""
        if self.focus.get("item") not in HEADER_FIELDS:
            self.click(HEADER["order-id"])

    def to_line(self, line, item="product-id"):
        """Click a line item. Clicking an empty row creates the line with the cursor in its first item, so click the
        item again (a second click in the item the cursor is in does nothing)."""
        position = (LINE[item], LINE_ROWS[line - 1])
        self.click(position)
        self.click(position)
        return position

    def leave(self):
        """Exit the form as a user, decided by what the form shows, never by timing: answer an open dialog (leave:
        Yes; save: No; a message: OK; anything else: No), else press Exit. When a failed validation or query mode
        keeps the form open after Exit, clear the record, then the block, and press Exit again."""
        keys = iter(["F4", "F4", "F6", "F4", "F7", "F4", "F4"])
        try:
            while self.alive():
                open_dialog = self.dialog()
                if open_dialog:
                    key = "Return" if "OK" in open_dialog["buttons"] else \
                        "y" if open_dialog["text"] == "Do you really want to leave the form?" else "n"
                else:
                    key = next(keys, None)
                    if key is None:
                        break
                xdotool("key", key)
                self.settle()
                self.wait_for_exit(2)
        finally:
            # The client exits by itself once the form has closed; signal it only when it does not.
            try:
                self.client.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.client.terminate()
                try:
                    self.client.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    self.client.kill()
                    self.client.wait()
            # The runtime ends after its client disconnects. Kill it only as a last resort, so its database
            # session and row locks always go.
            if not self.wait_for_exit(30):
                os.kill(self.runtime, 9)
                self.wait_for_exit(30)

    def alive(self):
        return os.path.exists("/proc/%d" % self.runtime)

    def wait_for_exit(self, seconds):
        deadline = time.time() + seconds
        while self.alive() and time.time() < deadline:
            time.sleep(0.1)
        return not self.alive()


def startup_failure(client_log):
    """The client's FRM- errors, to explain a failed launch."""
    try:
        with open(client_log, errors="replace") as log:
            return " | ".join([line.strip()[:400] for line in log if "FRM-" in line or "Exception:" in line][-12:])
    except OSError:
        return "no client log"


# The user's steps. Each works one session the way a user does; the examiner then observes the session.

def look(session):
    pass


def click(session, item, line=None):
    if line:
        session.to_line(line, item)
    else:
        session.click(HEADER[item])


def type_(session, item, value, line=None):
    if item in ("date-ordered", "date-shipped") and value:
        value = forms_date(value)
    position = session.to_line(line, item) if line else HEADER[item]
    session.replace(position, value)


def set_payment_type(session, payment_type):
    session.click(PAYMENT[payment_type])


def toggle(session, item):
    session.click(HEADER[item])


def next_item(session):
    session.keys("Tab")


def header_function(*keys):
    def function(session):
        session.to_header()
        session.keys(*keys)
    return function


def line_function(*keys):
    def function(session, line):
        session.to_line(line)
        session.keys(*keys)
    return function


def menu(name, item):
    def function(session):
        session.click(MENU[name])
        session.click(MENU_ITEMS[(name, item)])
    return function


def sort_by(session, field):
    menu("Sort By", field)(session)


def pick_product(session, line, product_id):
    """Open the product list of values on a line, find the product by its id and choose it."""
    session.to_line(line)
    session.keys("ctrl+l")
    xdotool("type", "--delay", 40, "--", str(product_id))
    session.keys("Return")  # find
    session.keys("Return")  # choose the first match


def answer(session, button):
    if session.dialog_open and session.dialog_open["buttons"] == ["OK"] and session.dialog_open["title"] == "Forms":
        session.boxes_seen += 1  # a message box: the next one shows the next unseen message
    session.keys(ANSWER_KEYS[button])


STEPS = {
    "look": look, "click": click, "type": type_, "set_payment_type": set_payment_type, "toggle": toggle,
    "next_item": next_item, "new_order": header_function("ctrl+Down"), "save": lambda session: session.keys("ctrl+s"),
    "enter_query": header_function("F11"), "execute_query": header_function("ctrl+F11"),
    "next_order": header_function("Down"), "previous_order": header_function("Up"),
    "delete_order": header_function("ctrl+Up"), "duplicate_order": header_function("shift+F6"),
    "clear": header_function("F7"), "cancel": menu("File", "Clear"), "exit": lambda session: session.click(EXIT_BUTTON),
    "delete_line": line_function("ctrl+Up"), "duplicate_line": line_function("shift+F6"),
    "pick_product": pick_product, "sort_by": sort_by, "answer": answer,
}
ACTIONS = sorted(STEPS)


def archive_dir():
    """STEP_ARCHIVE as a directory only the driver's user can enter, refusing one that someone else made."""
    os.makedirs(STEP_ARCHIVE, mode=0o700, exist_ok=True)
    status = os.lstat(STEP_ARCHIVE)
    if not stat.S_ISDIR(status.st_mode) or status.st_uid != os.geteuid():
        raise RuntimeError("step archive %s is not this user's directory" % STEP_ARCHIVE)
    os.chmod(STEP_ARCHIVE, 0o700)


class Case:
    """The sessions of one case, each started by its first step."""
    ended = 0  # cases this driver has ended, to name their step archives

    def __init__(self, name="case"):
        self.name = re.sub(r"[^A-Za-z0-9_.-]", "_", name)
        self.sessions = {}
        self.active = None

    def step(self, action, arguments):
        arguments = dict(arguments)
        name = arguments.pop("session", "A")
        session = self.sessions.get(name)
        if session is None:
            session = self.sessions[name] = Session(name)
        elif self.active is not session and session.alive():
            session.activate()
        self.active = session
        if not session.alive():
            return {"closed": True, "messages": []}  # the user has left the form; nothing more to do in it
        STEPS[action](session, **arguments)
        if action in ("exit", "answer") and session.wait_for_exit(3):
            log = session.text()
            session.steps.append((session.step_start, len(log)))
            return {"closed": True, "messages": distinct(frd.parse(log[session.step_start:]).messages)}
        return session.observe()

    def end(self):
        archive_dir()
        Case.ended += 1
        for name, session in sorted(self.sessions.items(), reverse=True):  # the second user leaves first
            if session.alive():
                if session is not self.active:
                    session.activate()  # the keyboard goes to the window in front
                    self.active = session
                session.leave()
            log = session.text()
            archive_name = "%s.%d-%d.%s.steps.log" % (self.name, os.getpid(), Case.ended, name)
            descriptor = os.open(os.path.join(STEP_ARCHIVE, archive_name),
                                 os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
            with os.fdopen(descriptor, "w") as archive:
                archive.write("".join(log[start:end] for start, end in session.steps))
        self.sessions = {}


def reply(message):
    sys.stdout.write(json.dumps(message, sort_keys=True) + "\n")
    sys.stdout.flush()


def main():
    reply({"protocol": 1, "driver": "summit-forms/2", "subject": os.environ.get("PARITY_SUBJECT", "legacy"),
           "actions": ACTIONS})
    case = Case()
    for line in sys.stdin:
        request = json.loads(line)
        kind, request_id = request.get("type"), request.get("request_id")
        try:
            if kind == "begin":
                case = Case(str(request.get("scenario_id") or "case"))
                value = None
            elif kind == "end":
                case.end()
                value = None
            else:
                value = case.step(request["action"], request.get("input") or {})
            reply({"type": "ok", "request_id": request_id, "value": value})
        except Exception as error:  # infrastructure failure, reported as such
            reply({"type": "error", "request_id": request_id, "code": type(error).__name__, "message": str(error)})


def run(steps):
    """One case from the command line: the observation after each step."""
    case, observations = Case("cli"), []
    try:
        for step in steps:
            observations.append(case.step(step["action"], step.get("input") or {}))
    except Exception as error:  # the steps so far, then why the next one could not be taken
        observations.append({"error": "%s: %s" % (type(error).__name__, error)})
    finally:
        case.end()
    return observations


if __name__ == "__main__" and len(sys.argv) == 1:
    main()
elif __name__ == "__main__":
    steps = json.loads(sys.argv[2]) if sys.argv[1] == "--steps" else \
        [{"action": sys.argv[1], "input": json.loads(sys.argv[2]) if len(sys.argv) > 2 else {}}]
    print(json.dumps(run(steps), indent=1, sort_keys=True))
