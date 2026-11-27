# summit-orders: the ORDERS form as a Rust and PostgreSQL web application

The migrated application, as the agent left it at the end of trial E11 r12 (nothing edited since). It passes 82 of
the 87 hidden journeys; see the repository README for the five it fails.

| File | What it does |
|---|---|
| `src/main.rs` | The web server (axum): the page, `/health`, and a session API. Each browser tab opens a session (`POST /api/session`), sends each user action to it (`POST /api/session/{id}/action`), keeps it alive with a heartbeat and closes it when the page goes. A session with no heartbeat for 12 seconds is dropped, which rolls back its transaction and releases its locks |
| `src/form.rs` | The form: one user session's orders and lines, record states, the cursor, item and record validation, Enter Query mode, Sort By, master-detail coordination, alerts, row locks (`select ... for update nowait`, compared with the row as fetched) and the save. Each session has its own PostgreSQL connection, as each Forms session has its own Oracle session |
| `src/index.html` | The page, built into the binary. It reports each event to the server and draws the screen the server returns |
| `schema.sql` | The PostgreSQL tables the form uses (Summit's table and column names, as the task required), the order sequence and the line audit trigger. No data |

## Build and run

```sh
cargo build --release --locked                     # produces target/release/summit-orders
createdb summit && psql -d summit -f schema.sql    # an empty database with the schema
# load the Summit business rows (regions, departments, employees, customers, products, orders, lines) into it
DATABASE_URL=postgres://USER@localhost/summit PORT=8080 ./target/release/summit-orders
# open http://localhost:8080
```

The application needs the Summit sample's business rows, which are Oracle's and not included. In the evaluation they
were copied from the Oracle database into PostgreSQL before each run (by a script of the evaluation harness,
Bosun's internal benchmark code, not published here).

The trial built it offline against vendored crates (`cargo build --locked --offline`); the versions are pinned in
`Cargo.lock`.
