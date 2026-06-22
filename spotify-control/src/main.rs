mod spotifyd_dbus;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::Parser;
use log::{error, info};
use serde::{Deserialize, Serialize};

use spotifyd_dbus::{Spotifyd, SpotifydError, Status};

#[derive(Parser)]
#[command(name = "spotify-control")]
#[command(about = "REST API to control a local spotifyd instance over D-Bus")]
struct Args {
    /// HTTP server bind address
    #[arg(long, default_value = "0.0.0.0:3001")]
    bind: String,
}

#[derive(Clone)]
struct AppState {
    spotifyd: Spotifyd,
}

#[derive(Serialize)]
struct OkResponse {
    ok: bool,
}

#[derive(Serialize)]
struct PauseResponse {
    ok: bool,
    paused: bool,
}

#[derive(Serialize)]
struct ErrorResponse {
    ok: bool,
    error: String,
}

#[derive(Deserialize)]
struct PlayRequest {
    uri: String,
}

#[derive(Deserialize)]
struct VolumeRequest {
    level: u8,
}

type ApiError = (StatusCode, Json<ErrorResponse>);

fn to_api_error(e: SpotifydError) -> ApiError {
    let status = match e {
        SpotifydError::NotRunning => StatusCode::SERVICE_UNAVAILABLE,
        SpotifydError::NotActive => StatusCode::CONFLICT,
        SpotifydError::ActivateTimeout => StatusCode::GATEWAY_TIMEOUT,
        SpotifydError::Dbus(_) | SpotifydError::Fdo(_) => StatusCode::BAD_GATEWAY,
    };
    (
        status,
        Json(ErrorResponse {
            ok: false,
            error: e.to_string(),
        }),
    )
}

async fn post_transfer(State(app): State<AppState>) -> Result<Json<OkResponse>, ApiError> {
    app.spotifyd.transfer_playback().await.map_err(to_api_error)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn post_play(
    State(app): State<AppState>,
    Json(req): Json<PlayRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    app.spotifyd.play_uri(&req.uri).await.map_err(to_api_error)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn post_pause(State(app): State<AppState>) -> Result<Json<PauseResponse>, ApiError> {
    let paused = app.spotifyd.pause_if_playing().await.map_err(to_api_error)?;
    Ok(Json(PauseResponse { ok: true, paused }))
}

async fn post_resume(State(app): State<AppState>) -> Result<Json<OkResponse>, ApiError> {
    app.spotifyd.resume().await.map_err(to_api_error)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn post_next(State(app): State<AppState>) -> Result<Json<OkResponse>, ApiError> {
    app.spotifyd.next().await.map_err(to_api_error)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn post_previous(State(app): State<AppState>) -> Result<Json<OkResponse>, ApiError> {
    app.spotifyd.previous().await.map_err(to_api_error)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn post_volume(
    State(app): State<AppState>,
    Json(req): Json<VolumeRequest>,
) -> Result<Json<OkResponse>, ApiError> {
    app.spotifyd
        .set_volume_percent(req.level)
        .await
        .map_err(to_api_error)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn get_status(State(app): State<AppState>) -> Result<Json<Status>, ApiError> {
    let status = app.spotifyd.status().await.map_err(to_api_error)?;
    Ok(Json(status))
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let args = Args::parse();

    let spotifyd = match Spotifyd::connect().await {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to connect to the session bus: {}", e);
            std::process::exit(1);
        }
    };

    let app = Router::new()
        .route("/api/spotify/transfer", post(post_transfer))
        .route("/api/spotify/play", post(post_play))
        .route("/api/spotify/pause", post(post_pause))
        .route("/api/spotify/resume", post(post_resume))
        .route("/api/spotify/next", post(post_next))
        .route("/api/spotify/previous", post(post_previous))
        .route("/api/spotify/volume", post(post_volume))
        .route("/api/spotify/status", get(get_status))
        .with_state(AppState { spotifyd });

    let listener = tokio::net::TcpListener::bind(&args.bind)
        .await
        .unwrap_or_else(|e| {
            error!("Failed to bind to {}: {}", args.bind, e);
            std::process::exit(1);
        });

    info!("spotify-control listening on {}", args.bind);

    axum::serve(listener, app).await.unwrap_or_else(|e| {
        error!("Server error: {}", e);
        std::process::exit(1);
    });
}
