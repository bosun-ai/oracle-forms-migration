"""What the Forms clients' screen shows that FRD does not log: whether a dialog is open (and how wide it is), and
which window belongs to which session.

Reading the screen means megabyte-sized X11 reads, which Parity's audit cannot capture whole, and window ids that
differ per run. So the examiner runs this as a small service outside the audited subject, like the X display itself
(`python3 screen.py serve`, started by legacy/container.sh and e5/environment.sh): the driver sends one-line requests
on a Unix socket and gets short, deterministic answers. Without the service the driver asks in process.

Requests: `dialog` -> the width of an open dialog's title bar, 0 when none; `mark` -> remember the windows now open;
`claim NAME` -> the one window opened since `mark` is session NAME's; `raise NAME` -> bring it to the front.
"""
import os
import socket
import struct
import subprocess
import sys

DISPLAY = os.environ.get("DISPLAY", ":99")
SOCKET = os.environ.get("FORMS_SCREEN_SOCKET", "/tmp/forms-screen.sock")
WINDOW_NAME = "Oracle Fusion Middleware Forms Services"
ACTIVE_TITLE = bytes((45, 132, 197))  # an active window's or dialog's title bar
WIDTH, HEIGHT = 760, 620  # the launcher window at the display's origin


def run(*argv):
    return subprocess.run(argv, stdout=subprocess.PIPE, check=True, env=dict(os.environ, DISPLAY=DISPLAY)).stdout


def dialog_title_width(raw):
    """The width of an open dialog's title bar in a WIDTH x HEIGHT RGB capture, or 0. With a dialog open, the form's
    own title bar is inactive."""
    row_bytes = WIDTH * 3
    if raw[41 * row_bytes + 300 * 3:41 * row_bytes + 303 * 3] == ACTIVE_TITLE:
        return 0
    widest = 0
    for y in range(60, 560):
        row, run_length = raw[y * row_bytes:(y + 1) * row_bytes], 0
        for x in range(0, row_bytes, 3):
            run_length = run_length + 1 if row[x:x + 3] == ACTIVE_TITLE else 0
            widest = max(widest, run_length)
    return widest


class Probe:
    def __init__(self):
        self.marked, self.windows = set(), {}

    def open_windows(self):
        return set(subprocess.run(["xdotool", "search", "--name", WINDOW_NAME], stdout=subprocess.PIPE,
                                  universal_newlines=True, env=dict(os.environ, DISPLAY=DISPLAY)).stdout.split())

    def handle(self, request):
        command, _, name = request.strip().partition(" ")
        if command == "dialog":
            return str(dialog_title_width(run("import", "-window", "root", "-crop", "%dx%d+0+0" % (WIDTH, HEIGHT),
                                              "-depth", "8", "rgb:-")))
        if command == "mark":
            self.marked = self.open_windows()
            return "ok"
        if command == "claim":
            new = self.open_windows() - self.marked
            if len(new) != 1:
                return "error: %d new windows" % len(new)
            self.windows[name] = new.pop()
            return "ok"
        if command == "raise":
            run("xdotool", "windowraise", self.windows[name])
            return "ok"
        return "error: unknown request"


LOCAL = Probe()


def ask(request):
    """The probe's answer, from the service when it runs, else in process."""
    if not os.path.exists(SOCKET):
        answer = LOCAL.handle(request)
    else:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.connect(SOCKET)
            connection.sendall((request + "\n").encode())
            answer = connection.makefile().readline().strip()
    if answer.startswith("error"):
        raise RuntimeError("screen probe: %s (%s)" % (answer, request))
    return answer


def peer_uid(connection):
    credentials = connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize("3i"))
    return struct.unpack("3i", credentials)[1]


def serve():
    """Answer the driver, which runs as this service's user; nobody else may connect (a graded port runs as another
    user in the same container): the socket is the user's only, and each peer's uid is checked."""
    if os.path.lexists(SOCKET):
        os.remove(SOCKET)
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as server:
        previous = os.umask(0o077)
        try:
            server.bind(SOCKET)
        finally:
            os.umask(previous)
        os.chmod(SOCKET, 0o600)
        server.listen()
        while True:
            connection, _ = server.accept()
            with connection:
                if peer_uid(connection) != os.geteuid():
                    continue
                stream = connection.makefile("rw")
                request = stream.readline()
                try:
                    answer = LOCAL.handle(request)
                except Exception as error:  # reported to the driver, which fails the step
                    answer = "error: %s" % error
                stream.write(answer + "\n")
                stream.flush()


if __name__ == "__main__" and sys.argv[1:] == ["serve"]:
    serve()
