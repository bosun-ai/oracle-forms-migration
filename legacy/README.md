# The legacy: Oracle's Summit ORDERS form

The legacy is Oracle's own Summit sample application, its ORDERS order-entry form: an order header (customer, sales
rep, dates, payment type, filled) with its order lines (product, price, quantity, shipped), validated by the form's
triggers and the Summit schema's constraints and triggers. It ran as Oracle ships it, on Oracle Forms 14.1.2
(WebLogic, the Forms server, the Java Forms client) and Oracle Database 23ai Free.

No Oracle software, form source, schema scripts, data or screenshots are in this repository. The video linked from
the top-level README shows the real Forms client.

## How it was driven

Through the real Java Forms client, under a virtual display, with keyboard and mouse: the driver
(`../proofs/driver/forms_legacy.py`) clicks items, types and presses the form's function keys as a user would. It
reads what the form shows from Forms Runtime Diagnostics traces and the dialogs from the display, and reports each
step's screen as JSON: the order, its lines, the item with the cursor, messages and alerts. After each journey,
`scripts/db/changes.sql` prints the rows the database committed.

## What you need from Oracle

Oracle's license terms apply to everything in this table. Each download needs you to accept them, and the
installers need an Oracle account.

| What | Where to get it | Where the build expects it |
|---|---|---|
| Forms 14.1.2 installer `V1045121-01.zip` and Fusion Middleware Infrastructure 14.1.2 `V1045135-01.zip` | [Oracle Forms Downloads](https://www.oracle.com/middleware/technologies/forms/downloads.html) (Forms and Reports 14.1.2.0.0, which requires Infrastructure 14.1.2.0.0); the links lead to [Oracle Software Delivery Cloud](https://edelivery.oracle.com/), where you sign in and accept the license | A directory passed as the `installers` build context (a directory of your choice) |
| Oracle JDK 17.0.12 | Downloaded by `scripts/Dockerfile` from download.oracle.com | Nothing to do |
| Oracle Database 23ai Free | [Oracle Database Free](https://www.oracle.com/database/free/) and its [container image](https://container-registry.oracle.com/ords/ocr/ba/database/free); we used `container-registry.oracle.com/database/free:latest-lite`. Oracle now names this release 26ai | A running database container with service `FREEPDB1` |
| Summit sample: `ORDERS.fmb`, `m_summit.mmb`, `calendar.pll`, `Form_Builder_II.olb` | Oracle's Summit sample for Forms (archive SHA-256 `709fdff8a33635fc48b2d09f3eb57582a879f0e85ef668acf6049c643ab5c44e`). Oracle distributes the Summit sample application with its Forms demos; we found no current Oracle download page for it, so obtain it from Oracle under Oracle's terms | One directory passed as the `forms-src` build context (a directory of your choice) |
| Summit schema export `init-summit.sql` | The same archive | Run once as user `summit` in `FREEPDB1` |

## How it was built

`scripts/` are the evaluation harness's build and run scripts (Bosun's internal benchmark code, not published here);
the Oracle installers and the Summit sources are passed to the builds as build contexts (see the comments at the top
of the Dockerfiles). `Runtime.Dockerfile` expects these scripts under a `legacy/` directory in its build context.

1. In the database, as SYSDBA in `FREEPDB1`: create user `summit` with connect, resource and the create view,
   synonym, type, procedure, trigger and sequence privileges; run `init-summit.sql` as `summit`.
2. `scripts/Dockerfile`: the Forms install image, built locally from your installers (response files in
   `scripts/install/`). Never push it.
3. `scripts/Runtime.Dockerfile`: that image plus the Summit sources, a WebLogic domain (`scripts/create_domain.py`,
   random admin password per build) and Parity.
4. `scripts/container.sh`: one container per run, on its own copy of the Summit schema (`scripts/db/copy_schema.sh`,
   random password), with the form compiled and the server started (`scripts/setup.sh`, `scripts/warmup.sh`).
5. Before each journey `scripts/db/reset.sql` restores the order tables and sequence.

## Security note

These scripts were written for a disposable, single-user audit container on a private machine, with a throwaway
schema copy and a random password per run. They are not hardened: the schema password appears on `sqlplus` command
lines and in the Forms web configuration inside that container, traces go to `/tmp`, and the screen helper trusts a
socket in `/tmp`. Do not run them on a shared or networked host, and do not reuse them with real credentials.
