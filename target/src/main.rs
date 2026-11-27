//! Embedded Summit order-entry application.
mod form;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::Html,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

#[derive(Clone)]
struct Application {
    database_url: Arc<String>,
    sessions: Arc<Mutex<HashMap<u64, SessionEntry>>>,
    next_session: Arc<AtomicU64>,
}

struct SessionEntry {
    form: Arc<Mutex<form::Session>>,
    last_seen: Instant,
}

type ApiResult = Result<Json<Value>, (StatusCode, Json<Value>)>;

fn api_error(error: impl std::fmt::Display) -> (StatusCode, Json<Value>) {
    eprintln!("{error}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "Database operation failed"})),
    )
}

async fn open_session(State(app): State<Application>) -> ApiResult {
    let mut session = form::Session::open(&app.database_url)
        .await
        .map_err(api_error)?;
    let id = app.next_session.fetch_add(1, Ordering::Relaxed);
    let mut screen = session.screen();
    screen["session"] = json!(id);
    app.sessions.lock().await.insert(
        id,
        SessionEntry {
            form: Arc::new(Mutex::new(session)),
            last_seen: Instant::now(),
        },
    );
    Ok(Json(screen))
}

async fn action(
    State(app): State<Application>,
    Path(id): Path<u64>,
    Json(command): Json<form::Command>,
) -> ApiResult {
    let session = {
        let mut sessions = app.sessions.lock().await;
        let entry = sessions.get_mut(&id).ok_or((
            StatusCode::NOT_FOUND,
            Json(json!({"error":"Session expired"})),
        ))?;
        entry.last_seen = Instant::now();
        entry.form.clone()
    };
    let mut session = session.lock().await;
    session.apply(command).await.map_err(api_error)?;
    Ok(Json(session.screen()))
}

async fn close_session(State(app): State<Application>, Path(id): Path<u64>) -> StatusCode {
    // Dropping the connection rolls back uncommitted edits and releases row locks.
    app.sessions.lock().await.remove(&id);
    StatusCode::NO_CONTENT
}

async fn heartbeat(State(app): State<Application>, Path(id): Path<u64>) -> StatusCode {
    if let Some(entry) = app.sessions.lock().await.get_mut(&id) {
        entry.last_seen = Instant::now();
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let database_url = std::env::var("DATABASE_URL")?;
    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "8080".into())
        .parse()?;
    let app = Application {
        database_url: Arc::new(database_url),
        sessions: Arc::new(Mutex::new(HashMap::new())),
        next_session: Arc::new(AtomicU64::new(1)),
    };
    let sessions = app.sessions.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            // Browser termination need not dispatch pagehide. Expired leases drop connections and roll back.
            sessions
                .lock()
                .await
                .retain(|_, entry| entry.last_seen.elapsed() < Duration::from_secs(12));
        }
    });
    let router = Router::new()
        .route("/", get(|| async { Html(include_str!("index.html")) }))
        .route("/health", get(|| async { "OK" }))
        .route("/api/session", post(open_session))
        .route("/api/session/{id}/action", post(action))
        .route("/api/session/{id}/close", post(close_session))
        .route("/api/session/{id}/heartbeat", post(heartbeat))
        .with_state(app);
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port)).await?;
    axum::serve(listener, router).await?;
    Ok(())
}
