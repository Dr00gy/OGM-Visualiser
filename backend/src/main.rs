//! # OGM Visualiser Backend
//!
//! HTTP server that accepts 2–3 groups of XMAP files (one group per genome),
//! parses them, finds query-sequence matches that appear across all genomes,
//! and streams the resolved matches back to the frontend as a length-prefixed
//! `bincode` byte stream.
//!
//! ## Module layout
//! * [`xmap`] — all parsing, indexing, caching, and the parallel match engine.
//! * [`api`] — HTTP handlers: session creation, per-file uploads, and the
//!             streaming match endpoint.
//! * [`store`] — columnar match store and aggregate index.
//! * [`query`] — read-only HTTP handlers for the populated store.
//!
//! ## Request flow (session-based)
//! Files are uploaded one at a time against a server-side session so that the
//! browser never has to hold a single multi-gigabyte multipart body in memory
//! (which previously stalled uploads around ~500 MiB of RAM pressure).
//!
//! 1. Client `POST /api/session` → server allocates a fresh session UUID and
//!    a staging directory under the OS temp dir; returns the UUID.
//! 2. For each file in each genome, client `POST /api/upload/:session_id`
//!    with a multipart body containing exactly ONE field named using the
//!    `g{gi}_r` (refineFinal) / `g{gi}_s{fi}` (sequence/xmap) convention.
//!    The server streams the field to disk in the staging directory and
//!    records it in the session.
//! 3. Once every file is uploaded, client `POST /api/match/:session_id` to
//!    kick off processing. The response is the length-prefixed bincode stream
//!    described in [`api`]. The session (and its staging dir) is deleted
//!    when the stream completes or the client disconnects.
//! 4. If the client cancels before `match`, it can `DELETE /api/session/:id`
//!    to clean up immediately.

use std::sync::Arc;
use axum::{routing::{get, post, delete}, Router};
use axum::extract::DefaultBodyLimit;
use tower_http::cors::{CorsLayer, Any};

mod xmap;
mod api;
mod store;
mod query;

use api::AppState;

async fn root() -> &'static str {
    "XMAP Backend server is running!\n\n\
     Endpoints:\n\
     - GET    /\n\
     - POST   /api/session\n\
     - POST   /api/upload/{session_id}\n\
     - POST   /api/match/{session_id}\n\
     - DELETE /api/session/{session_id}\n\
     - GET    /api/session/{session_id}/meta\n\
     - GET    /api/session/{session_id}/sequences\n\
     - GET    /api/session/{session_id}/matches\n\
     - GET    /api/session/{session_id}/flows\n\
     - GET    /api/session/{session_id}/chromosome-records\n\
     - GET    /api/session/{session_id}/sequence-locations"
}

/// Application entry point.
///
/// # Setup steps
/// 1. Build the shared [`xmap::XmapCache`] and [`api::SessionStore`], bundle
///    them into an [`AppState`] wrapped in an [`Arc`] so every request handler
///    sees the same cache/session state. Parsing a multi-gigabyte XMAP file
///    is expensive; caching by content hash lets repeated uploads of the
///    same file reuse prior work. The session store tracks each in-progress
///    upload's staging directory and per-genome file list.
/// 2. Configure a permissive CORS layer for the Vite dev server on
///    `localhost:5173` (both `localhost` and `127.0.0.1` variants). In
///    production this should be tightened to the real frontend origin.
/// 3. Register routes and attach the state via `.with_state`.
/// 4. Raise the default body-size limit on the upload route specifically —
///    individual XMAP files can easily exceed axum's default 2 MiB cap. We
///    set a generous 4 GiB ceiling (single-file, not total across the session)
///    which is well above any realistic refineFinal or sequence file.
/// 5. Bind on `0.0.0.0:8080` so the server is reachable from other hosts
///    on the LAN (useful for demoing on a different machine than the one
///    running `cargo run`).
#[tokio::main]
async fn main() {
    let state = Arc::new(AppState {
        cache: Arc::new(xmap::XmapCache::new()),
        sessions: Arc::new(api::SessionStore::new()),
    });

    api::spawn_session_janitor(Arc::clone(&state.sessions));

    let cors = CorsLayer::new()
        .allow_origin([
            "http://localhost:5173".parse().unwrap(),
            "http://127.0.0.1:5173".parse().unwrap(),
        ])
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/", get(root))
        .route("/api/session", post(api::create_session))
        .route(
            "/api/upload/{session_id}",
            post(api::upload_file)
                .layer(DefaultBodyLimit::max(4 * 1024 * 1024 * 1024)), // 4 GiB
        )
        .route("/api/match/{session_id}", post(api::stream_matches))
        .route("/api/session/{session_id}", delete(api::delete_session))
        .route("/api/session/{session_id}/meta",      get(query::get_meta))
        .route("/api/session/{session_id}/sequences", get(query::get_sequences))
        .route("/api/session/{session_id}/matches",   get(query::get_matches))
        .route("/api/session/{session_id}/flows",     get(query::get_flows))
        .route("/api/session/{session_id}/chromosome-records",
               get(query::get_chromosome_records))
        .route("/api/session/{session_id}/sequence-locations",
               get(query::get_sequence_locations))
        .with_state(state)
        .layer(cors)
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .unwrap();

    println!("Server running on http://0.0.0.0:8080");
    axum::serve(listener, app).await.unwrap();
}