# The Parity proofs

This is the Parity project the agent wrote while it ported the form (trial E11 r12), as it was at the end of the
run. Parity runs each proof as a user journey on two subjects, the legacy (real Oracle Forms) and the target (the
port), and compares what each did. A proof passes only when both sides are equal and its assertions hold.

| File | What it is |
|---|---|
| `Parityfile` | The two subjects, the `changes` view, and the audit policy |
| `specs/orders-end-to-end.md` | The 6 proofs (80 steps) |
| `Parityfile.lock` | The locked proofs: Parity refuses a proof that changed since it was audited |
| `bin/` | The commands the Parityfile names: reset a database, print its changes, start a driver |
| `driver/` | The Forms driver: keyboard and mouse on the Forms client, the screen read from Forms Runtime Diagnostics |
| `evidence/plan.txt` | Parity's plan of the final check: each proof's steps, views and audit rules |
| `evidence/final-check.json`, `evidence/final-check-verify.json` | The workflow's final check (build, tests, a fresh `parity verify --no-cache`, an offline replay) and Parity's verdicts: 6 passed, 0 failed |

The captured evidence (Parity's recordings of both sides) is not included. The paths `/u01/...` and
`/workspace/dst/...` are inside the trial container: `/u01/legacy` holds the scripts in `../legacy/scripts`, and
`/u01/target/bin` the evaluation harness's target tooling (PostgreSQL reset and changes, and a Playwright driver for the port
with the same actions as the Forms driver; Bosun's internal benchmark code, not published here).

## How a proof reads

```parity
proof orders-end-to-end.explicit-query-edit {
  when entered = enter_query()
  when criteria = type(item = "order-id", value = 101)
  when queried = execute_query()
  when changed = type(item = "customer-id", value = 201)
  when left = click(item = "date-shipped")
  when saved = save()
  then {
    assert steps.entered.mode == "query"
    assert steps.queried.order["order-id"] == 101
    assert steps.changed.order["customer-name"] == "Kam's Sporting Goods"
    assert steps.left.order["customer-name"] == "Unisports"
    assert result.order["rep-id"] == 12
    assert result.messages == ["FRM-40400: Transaction complete: 1 records applied and saved."]
  }
}
```

1. **Before the journey**, Parity resets both databases to the same baseline: on the legacy `bin/reset.sh legacy`
   (Oracle: restore the order tables and the order sequence); on the target `/u01/target/bin/reset.sh` (PostgreSQL,
   loaded from the same rows).
2. **The legacy command.** Parity starts `bin/driver.sh legacy`, which runs `driver/forms_legacy.py` against the
   Forms server. Each `when` line is one user action: Enter Query, type 101 into Order Id, Execute Query, type 201
   into Customer Id, click Date Shipped, Save. After each action the driver reports the screen as JSON: the mode,
   the order header, its lines, the item that holds the cursor, the messages and any alert.
3. **The target command.** Parity starts the port (`serve`: the built `summit-orders` binary with `DATABASE_URL`,
   ready when `/health` answers) and `bin/web-driver.sh target`, a Playwright driver that performs the same six
   actions in a browser.
4. **What is compared.**
   - Every step's screen, legacy against target, field by field. Here: after typing 201 the customer name is still
     the old one ("Kam's Sporting Goods") because Forms validates the item only when the cursor leaves it; after the
     click it is "Unisports". A port that looked the name up while typing would fail at step `changed`.
   - The `changes` view after the journey: the rows each database committed, against its baseline
     (`bin/changes.sh`), so the saved order must be the same row on both sides.
   - Everything else the subjects did, under `audit required`: files, processes, network. The policy at the end of
     the `Parityfile` names what may differ and why (for example, the Forms client's traffic to its server, and the
     browser driver's own connections). Anything not explained there fails the proof, and incomplete evidence gives
     no verdict.
5. **The assertions** (`then`) state what the agent expected to see; they must hold on both sides. They make the
   proof's intent readable, but the comparison above is what ties the port to the legacy.

The other five proofs cover Sort By with the Immediate and Auto Query options, refused header entries (an unknown
customer, a cash-only customer choosing credit, a ship date before the order date), a new order with two lines,
clearing and leaving the form with unsaved changes, and duplicating and deleting an order.

## Security note

These scripts were written for a disposable, single-user audit container on a private machine, with a throwaway
schema copy and a random password per run. They are not hardened: the schema password appears on `sqlplus` command
lines and in the Forms web configuration inside that container, traces go to `/tmp`, and the screen helper trusts a
socket in `/tmp`. Do not run them on a shared or networked host, and do not reuse them with real credentials.
