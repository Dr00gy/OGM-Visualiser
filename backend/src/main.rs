//! # OGM Visualiser Backend
//!
//! HTTP server that accepts 2–3 groups of XMAP files (one group per genome),
//! parses them, finds query-sequence matches that appear across all genomes,
//! and streams the resolved matches back to the frontend.
//!
//! Sessions are used so the browser uploads files individually rather than
//! holding a multi-gigabyte multipart body in memory:
//! 1. `POST /api/session` → fresh session UUID + staging dir
//! 2. `POST /api/upload/:session_id` per file (one multipart field named
//!    `g{gi}_r` for refineFinal, `g{gi}_s{fi}` for sequence/xmap)
//! 3. `POST /api/match/:session_id` to start processing; response is the
//!    length-prefixed bincode stream (see [`api`])
//! 4. `DELETE /api/session/:id` to clean up before match

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

#[tokio::main]
async fn main() {
    let state = Arc::new(AppState {
        cache: Arc::new(xmap::XmapCache::new()),
        sessions: Arc::new(api::SessionStore::new()),
    });

    api::spawn_session_janitor(Arc::clone(&state.sessions), Arc::clone(&state.cache));

    // Permissive CORS for the Vite dev server.
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
                .layer(DefaultBodyLimit::max(4 * 1024 * 1024 * 1024)),
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