use audio::led_engine::{LedEngine, LedEvent, LedState};
use audio::led_ring::LedRing;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use log::{error, info};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tower_http::cors::CorsLayer;

#[derive(Parser)]
#[command(name = "led_controller")]
#[command(about = "HTTP server for controlling the ReSpeaker LED ring")]
struct Args {
    /// HTTP server bind address
    #[arg(long, default_value = "0.0.0.0:3000")]
    bind: String,

    /// I2C bus device path
    #[arg(long, default_value = "/dev/i2c-1")]
    i2c_bus: String,
}

#[derive(Clone)]
struct AppState {
    event_tx: mpsc::Sender<LedEvent>,
    engine_state: Arc<Mutex<EngineSnapshot>>,
}

#[derive(Clone, Serialize)]
struct EngineSnapshot {
    state: LedState,
    volume: u8,
}

#[derive(Serialize)]
struct EventResponse {
    ok: bool,
}

#[derive(Serialize)]
struct ErrorResponse {
    ok: bool,
    error: String,
}

async fn post_event(
    State(app): State<AppState>,
    Json(event): Json<LedEvent>,
) -> Result<Json<EventResponse>, (StatusCode, Json<ErrorResponse>)> {
    app.event_tx.send(event).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                ok: false,
                error: format!("Engine channel closed: {}", e),
            }),
        )
    })?;

    Ok(Json(EventResponse { ok: true }))
}

async fn get_status(State(app): State<AppState>) -> Json<EngineSnapshot> {
    let snapshot = app.engine_state.lock().await.clone();
    Json(snapshot)
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let args = Args::parse();

    info!("Opening I2C bus: {}", args.i2c_bus);
    let ring = match LedRing::new(&args.i2c_bus) {
        Ok(ring) => ring,
        Err(e) => {
            error!("Failed to initialize LED ring: {}", e);
            std::process::exit(1);
        }
    };

    let (event_tx, event_rx) = mpsc::channel::<LedEvent>(32);

    let engine_state = Arc::new(Mutex::new(EngineSnapshot {
        state: LedState::Idle,
        volume: 50,
    }));

    let engine_state_writer = Arc::clone(&engine_state);
    tokio::spawn(async move {
        let mut engine = LedEngine::new(ring, event_rx);
        loop {
            engine.run_tick().await;

            let mut snapshot = engine_state_writer.lock().await;
            snapshot.state = engine.state();
            snapshot.volume = engine.volume_percent();
        }
    });

    let app_state = AppState {
        event_tx,
        engine_state,
    };

    let app = Router::new()
        .route("/api/event", post(post_event))
        .route("/api/status", get(get_status))
        .layer(CorsLayer::permissive())
        .with_state(app_state);

    let listener = tokio::net::TcpListener::bind(&args.bind)
        .await
        .unwrap_or_else(|e| {
            error!("Failed to bind to {}: {}", args.bind, e);
            std::process::exit(1);
        });

    info!("LED controller listening on {}", args.bind);

    axum::serve(listener, app)
        .await
        .unwrap_or_else(|e| {
            error!("Server error: {}", e);
            std::process::exit(1);
        });
}
