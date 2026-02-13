use crate::config::Config;
use crate::state::AppState;
use tokio_util;
use crate::types::{Message, Attachment, EVENT_MESSAGE, EVENT_OPEN};
use axum::{
    body::Body,
    extract::{Path, State, Query, ConnectInfo},
    http::{StatusCode, HeaderMap},
    response::{IntoResponse, sse::{Event, Sse}},
    routing::{get, post},
    Json, Router,
};
use futures::stream::StreamExt;
use std::{convert::Infallible, sync::Arc, time::Duration};
use std::collections::HashMap;
use crate::util;
use tokio_stream::wrappers::BroadcastStream;
use tracing::{info, error, instrument};
use uuid::Uuid;
use chrono::Utc;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use crate::types::Action;
use url::Url;

use crate::store::Store;

use crate::file_storage::FileStorage;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::extract::FromRef;
use crate::auth::types::{User, Role, Permission, Tier};
use base64::{Engine as _, engine::general_purpose};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "client/"]
struct Assets;

#[derive(Debug)]
pub struct Auth(pub User);

impl<S> FromRequestParts<S> for Auth
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        let auth_manager = &state.auth_manager;

        // 1. Check Authorization header
        if let Some(auth_header) = parts.headers.get("Authorization") {
            if let Ok(auth_str) = auth_header.to_str() {
                if auth_str.starts_with("Basic ") {
                    let token = &auth_str[6..];
                    if let Ok(decoded) = general_purpose::STANDARD.decode(token) {
                        if let Ok(creds) = String::from_utf8(decoded) {
                            if let Some((username, password)) = creds.split_once(':') {
                                if let Ok(Some(user)) = auth_manager.authenticate(username, password).await {
                                    return Ok(Auth(user));
                                }
                            }
                        }
                    }
                } else if auth_str.starts_with("Bearer ") {
                     let token = &auth_str[7..];
                     if let Ok(Some(user)) = auth_manager.authenticate_token(token).await {
                         return Ok(Auth(user));
                     }
                }
            }
        }

        // 2. Check query param "auth"
        if let Some(query) = parts.uri.query() {
            if let Ok(params) = serde_urlencoded::from_str::<HashMap<String, String>>(query) {
                if let Some(token) = params.get("auth") {
                     let token_str: &str = token;
                     if let Ok(Some(user)) = auth_manager.authenticate_token(token_str).await {
                         return Ok(Auth(user));
                     }
                }
            }
        }

        // Default to Anonymous user
        Ok(Auth(User {
            id: "u_everyone".to_string(),
            name: "*".to_string(),
            hash: "".to_string(),
            role: Role::Anonymous,
            tier: Tier::Free,
        }))
    }
}

pub async fn run(config: Config) -> anyhow::Result<()> {
    run_with_addr(config, None).await
}

pub async fn run_with_addr(config: Config, addr_tx: Option<tokio::sync::oneshot::Sender<std::net::SocketAddr>>) -> anyhow::Result<()> {
    let addr = config.listen_http.clone();
    let cache_file = config.cache_file.clone();
    let attachment_dir = config.attachment_cache_dir.clone();
    let attachment_limit = config.total_attachment_size_limit;
    
    use crate::auth::manager::AuthManager;
    use crate::email::sender::Mailer;
    use crate::email::server::SmtpServer;
    use crate::cron::Manager as CronManager;
    use crate::webpush::WebPushStore;
    use crate::firebase::FirebaseClient;

    let store = if cache_file.as_os_str().is_empty() || cache_file.to_string_lossy() == ":memory:" {
        Store::new(None).await?
    } else {
        Store::new(Some(std::path::Path::new(&cache_file))).await?
    };
    let file_storage = FileStorage::new(&attachment_dir, attachment_limit as u64).await?;
    let config_arc = Arc::new(config.clone());
    
    let auth_pool = sqlx::SqlitePool::connect(&format!("sqlite://{}?mode=rwc", config.auth_file.display())).await?;
    sqlx::migrate!("./migrations").run(&auth_pool).await?;

    let auth_manager = AuthManager::new(auth_pool.clone(), config_arc.clone()).await?;

    let webpush = if !config.web_push_public_key.is_empty() {
        let wp_pool = if config.web_push_file == config.auth_file {
            auth_pool.clone()
        } else {
            sqlx::SqlitePool::connect(&format!("sqlite://{}?mode=rwc", config.web_push_file.display())).await?
        };
        Some(WebPushStore::new(wp_pool, config_arc.clone()).await?)
    } else {
        None
    };

    let firebase = if !config.firebase_key_file.as_os_str().is_empty() {
        match FirebaseClient::new(config_arc.clone()).await {
            Ok(fb) => Some(fb),
            Err(e) => {
                error!("Failed to initialize Firebase: {}", e);
                None
            }
        }
    } else {
        None
    };

    let mailer = Mailer::new(config_arc.clone());
    let limiter = crate::limits::Limiter::new(
        config.visitor_request_limit_burst as u32, 
        500,
        config.visitor_attachment_daily_bandwidth_limit
    );

    let state = AppState::new(config, store, file_storage, auth_manager, mailer, limiter, webpush, firebase);

    let app = Router::new()
        .route("/v1/health", get(handle_health))
        .route("/v1/config", get(handle_config))
        .route("/v1/webpush", post(handle_webpush_update).delete(handle_webpush_delete))
        .route("/config.js", get(handle_web_config))
        .route("/file/:id", get(handle_file))
        .route("/_matrix/push/v1/notify", get(handle_matrix_discovery).post(handle_matrix_push))
        .route("/:topic", post(handle_publish).put(handle_publish).get(handle_subscribe_sse))
        .route("/:topic/json", get(handle_subscribe_json))
        .route("/:topic/sse", get(handle_subscribe_sse))
        .route("/:topic/es", get(handle_subscribe_sse))
        .route("/:topic/eventsource", get(handle_subscribe_sse))
        .route("/:topic/raw", get(handle_subscribe_raw))
        .route("/:topic/publish", post(handle_publish).put(handle_publish))
        .route("/:topic/send", post(handle_publish).put(handle_publish))
        .route("/:topic/trigger", post(handle_publish).put(handle_publish))
        .route("/", post(handle_publish).put(handle_publish).get(handle_static_root))
        .fallback(handle_static)
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state.clone());

    let smtp_server = SmtpServer::new(state.clone());
    tokio::spawn(async move {
        if let Err(e) = smtp_server.run().await {
            tracing::error!("SMTP server error: {}", e);
        }
    });

    let cron_manager = CronManager::new(state.clone());
    tokio::spawn(async move {
        cron_manager.run().await;
    });

    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bound_addr = listener.local_addr()?;
    if let Some(tx) = addr_tx {
        let _ = tx.send(bound_addr);
    }

    info!("Listening on {}", bound_addr);
    axum::serve(listener, app).await?;

    Ok(())
}

async fn handle_health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "healthy": true }))
}

async fn handle_config(State(state): State<AppState>) -> Json<Config> {
    Json((*state.config).clone())
}

async fn handle_web_config(State(state): State<AppState>) -> impl IntoResponse {
    let config_json = serde_json::to_string(&*state.config).unwrap_or_default();
    let js = format!("// Generated server configuration\nvar config = {};\n", config_json);
    (
        StatusCode::OK,
        [("Content-Type", "text/javascript"), ("Cache-Control", "no-cache")],
        js,
    )
}

async fn handle_static_root() -> impl IntoResponse {
    handle_static_path("index.html".to_string())
}

async fn handle_static(uri: axum::http::Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/').to_string();
    handle_static_path(path)
}

fn handle_static_path(path: String) -> impl IntoResponse {
    let path = if path.is_empty() { "index.html".to_string() } else { path };
    
    match Assets::get(&path) {
        Some(content) => {
            let body = Body::from(content.data);
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            (
                StatusCode::OK,
                [("Content-Type", mime.as_ref())],
                body,
            ).into_response()
        }
        None => {
            if let Some(content) = Assets::get("index.html") {
                let body = Body::from(content.data);
                (
                    StatusCode::OK,
                    [("Content-Type", "text/html")],
                    body,
                ).into_response()
            } else {
                StatusCode::NOT_FOUND.into_response()
            }
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct MatrixRequest {
    pub notification: Option<MatrixNotification>,
}

#[derive(Deserialize, Debug)]
pub struct MatrixNotification {
    pub devices: Option<Vec<MatrixDevice>>,
}

#[derive(Deserialize, Debug)]
pub struct MatrixDevice {
    pub pushkey: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MatrixResponse {
    pub rejected: Vec<String>,
}

async fn handle_matrix_discovery() -> impl IntoResponse {
    (
        StatusCode::OK,
        [("Content-Type", "application/json")],
        r#"{"unifiedpush":{"gateway":"matrix"}}"#,
    )
}

#[instrument(skip(state, body))]
async fn handle_matrix_push(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Query<HashMap<String, String>>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    auth: Auth,
    body: Body,
) -> Result<impl IntoResponse, impl IntoResponse> {
    let bytes = match axum::body::to_bytes(body, state.config.message_size_limit * 2).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("Failed to read Matrix body: {}", e);
            return Err((StatusCode::BAD_REQUEST, "Failed to read body").into_response());
        }
    };

    let req: MatrixRequest = match serde_json::from_slice(&bytes) {
        Ok(r) => r,
        Err(_) => return Err((StatusCode::BAD_REQUEST, "Invalid JSON").into_response()),
    };

    let pushkey = match req.notification.and_then(|n| n.devices).and_then(|d| d.first().map(|dev| dev.pushkey.clone())) {
        Some(p) => p,
        None => return Err((StatusCode::BAD_REQUEST, "Missing pushkey").into_response()),
    };

    if !pushkey.starts_with(&state.config.base_url) {
        return Ok((
            StatusCode::OK,
            Json(MatrixResponse { rejected: vec![pushkey] }),
        ).into_response());
    }

    let url: Url = match Url::parse(&pushkey) {
        Ok(u) => u,
        Err(_) => return Err((StatusCode::BAD_REQUEST, "Invalid pushkey URL").into_response()),
    };

    let topic = url.path().trim_start_matches('/').to_string();
    let query_params: HashMap<String, String> = url.query_pairs().into_owned().collect();

    let topic_obj = state.get_topic(&topic);
    if let Ok(elapsed) = topic_obj.last_access.elapsed() {
        if elapsed.as_secs() > 12 * 3600 {
            return Ok((
                StatusCode::OK,
                Json(MatrixResponse { rejected: vec![pushkey] }),
            ).into_response());
        }
    }

    Ok(handle_publish(
        State(state),
        Some(Path(topic)),
        headers,
        Query(query_params),
        ConnectInfo(addr),
        auth,
        Body::from(bytes.clone()),
    ).await.into_response())
}

#[derive(Deserialize, Debug)]
pub struct WebPushUpdateRequest {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    pub topics: Vec<String>,
}

async fn handle_webpush_update(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    auth: Auth,
    Json(req): Json<WebPushUpdateRequest>,
) -> impl IntoResponse {
    let wp = match &state.webpush {
        Some(wp) => wp,
        None => return StatusCode::NOT_IMPLEMENTED.into_response(),
    };

    let user = auth.0;
    for topic in &req.topics {
        match state.auth_manager.authorize(&user, topic, Permission::Read).await {
            Ok(true) => {},
            _ => return (StatusCode::FORBIDDEN, format!("Access to topic {} denied", topic)).into_response(),
        }
    }

    match wp.upsert_subscription(
        &req.endpoint,
        &req.auth,
        &req.p256dh,
        if user.role == Role::Anonymous { None } else { Some(&user.id) },
        &addr.ip().to_string(),
        &req.topics,
    ).await {
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => {
            tracing::error!("Failed to update WebPush subscription: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn handle_webpush_delete(
    State(state): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let wp = match &state.webpush {
        Some(wp) => wp,
        None => return StatusCode::NOT_IMPLEMENTED.into_response(),
    };

    let endpoint = match req.get("endpoint").and_then(|e| e.as_str()) {
        Some(e) => e,
        None => return (StatusCode::BAD_REQUEST, "Missing endpoint").into_response(),
    };

    match wp.remove_subscription(endpoint).await {
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => {
            tracing::error!("Failed to delete WebPush subscription: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct PublishRequest {
    pub topic: Option<String>,
    pub message: Option<String>,
    pub title: Option<String>,
    pub priority: Option<i32>,
    pub tags: Option<Vec<String>>,
    pub click: Option<String>,
    pub icon: Option<String>,
    pub actions: Option<Vec<Action>>,
    pub attachment: Option<String>,
    pub attach: Option<String>,
    pub file: Option<String>,
    pub filename: Option<String>,
    pub email: Option<String>,
    pub delay: Option<String>,
    pub at: Option<String>,
    #[serde(rename = "in")]
    pub in_str: Option<String>,
}

#[instrument(skip(state, body))]
async fn handle_publish(
    State(state): State<AppState>,
    topic_path: Option<Path<String>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    auth: Auth,
    body: Body,
) -> impl IntoResponse {
    if !state.limiter.check(addr.ip()) {
         return (StatusCode::TOO_MANY_REQUESTS, "Limit reached").into_response();
    }

    let id = Uuid::new_v4().to_string().chars().take(12).collect::<String>();
    let mut topic_id = topic_path.map(|Path(t)| t);
    
    let mut title = headers.get("Title").and_then(|h| h.to_str().ok()).map(|s| s.to_string())
        .or_else(|| headers.get("X-Title").and_then(|h| h.to_str().ok()).map(|s| s.to_string()));
        
    let mut priority = headers.get("Priority").and_then(|h| h.to_str().ok()).and_then(|s| s.parse().ok())
        .or_else(|| headers.get("X-Priority").and_then(|h| h.to_str().ok()).and_then(|s| s.parse().ok()));

    let mut tags = headers.get("Tags").and_then(|h| h.to_str().ok()).map(|s| s.split(',').map(|s| s.trim().to_string()).collect())
        .or_else(|| headers.get("X-Tags").and_then(|h| h.to_str().ok()).map(|s| s.split(',').map(|s| s.trim().to_string()).collect()));

    let mut click = headers.get("Click").and_then(|h| h.to_str().ok()).map(|s| s.to_string())
        .or_else(|| headers.get("X-Click").and_then(|h| h.to_str().ok()).map(|s| s.to_string()));

    let mut icon = headers.get("Icon").and_then(|h| h.to_str().ok()).map(|s| s.to_string())
        .or_else(|| headers.get("X-Icon").and_then(|h| h.to_str().ok()).map(|s| s.to_string()));

    let mut actions = headers.get("Actions").and_then(|h| h.to_str().ok()).and_then(|s| {
        util::parse_actions(s).ok()
    }).or_else(|| {
        headers.get("X-Actions").and_then(|h| h.to_str().ok()).and_then(|s| {
            util::parse_actions(s).ok()
        })
    });

    let mut filename = headers.get("Filename").and_then(|h| h.to_str().ok()).map(|s| s.to_string())
        .or_else(|| headers.get("X-Filename").and_then(|h| h.to_str().ok()).map(|s| s.to_string()))
        .or_else(|| headers.get("File").and_then(|h| h.to_str().ok()).map(|s| s.to_string()))
        .or_else(|| headers.get("X-File").and_then(|h| h.to_str().ok()).map(|s| s.to_string()));

    let mut email_addr = headers.get("Email").and_then(|h| h.to_str().ok()).map(|s| s.to_string())
        .or_else(|| headers.get("X-Email").and_then(|h| h.to_str().ok()).map(|s| s.to_string()))
        .or_else(|| params.get("email").map(|s| s.to_string()));

    let mut delay_str = headers.get("Delay").and_then(|h| h.to_str().ok()).map(|s| s.to_string())
        .or_else(|| headers.get("X-Delay").and_then(|h| h.to_str().ok()).map(|s| s.to_string()))
        .or_else(|| headers.get("At").and_then(|h| h.to_str().ok()).map(|s| s.to_string()))
        .or_else(|| headers.get("X-At").and_then(|h| h.to_str().ok()).map(|s| s.to_string()))
        .or_else(|| headers.get("In").and_then(|h| h.to_str().ok()).map(|s| s.to_string()))
        .or_else(|| headers.get("X-In").and_then(|h| h.to_str().ok()).map(|s| s.to_string()))
        .or_else(|| params.get("delay").map(|s| s.to_string()))
        .or_else(|| params.get("at").map(|s| s.to_string()))
        .or_else(|| params.get("in").map(|s| s.to_string()));

    let mut message_body = None;
    let mut attachment_url = headers.get("Attach").and_then(|h| h.to_str().ok()).map(|s| s.to_string())
        .or_else(|| headers.get("X-Attach").and_then(|h| h.to_str().ok()).map(|s| s.to_string()));

    let mut bytes = None;
    let mut json_request = None;
    let mut body_opt = Some(body);

    if let Some("application/json") = headers.get("Content-Type").and_then(|h| h.to_str().ok()) {
        let b = match axum::body::to_bytes(body_opt.take().unwrap(), state.config.message_size_limit * 2).await {
            Ok(b) => b,
            Err(_) => return (StatusCode::BAD_REQUEST, "Failed to read body").into_response(),
        };
        if let Ok(req) = serde_json::from_slice::<PublishRequest>(&b) {
            if let Some(t) = &req.topic { topic_id = Some(t.clone()); }
            if req.message.is_some() { message_body = req.message.clone(); }
            if req.title.is_some() { title = req.title.clone(); }
            if req.priority.is_some() { priority = req.priority; }
            if req.tags.is_some() { tags = req.tags.clone(); }
            if req.click.is_some() { click = req.click.clone(); }
            if req.icon.is_some() { icon = req.icon.clone(); }
            if req.actions.is_some() { actions = req.actions.clone(); }
            if req.attachment.is_some() { attachment_url = req.attachment.clone(); }
            if req.attach.is_some() { attachment_url = req.attach.clone(); }
            if req.file.is_some() { attachment_url = req.file.clone(); }
            if req.filename.is_some() { filename = req.filename.clone(); }
            if req.email.is_some() { email_addr = req.email.clone(); }
            if req.delay.is_some() { delay_str = req.delay.clone(); }
            if req.at.is_some() { delay_str = req.at.clone(); }
            if req.in_str.is_some() { delay_str = req.in_str.clone(); }
            json_request = Some(req);
        }
        bytes = Some(b);
    }

    let topic_id = match topic_id {
        Some(t) => t,
        None => return (StatusCode::BAD_REQUEST, "Missing topic").into_response(),
    };

    let user = auth.0;
    match state.auth_manager.authorize(&user, &topic_id, Permission::Write).await {
        Ok(true) => {},
        Ok(false) => {
            if user.role == Role::Anonymous {
                return (StatusCode::UNAUTHORIZED, [("WWW-Authenticate", "Basic realm=\"ntfy\"")], "Unauthorized").into_response();
            } else {
                return (StatusCode::FORBIDDEN, "Forbidden").into_response();
            }
        },
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }

    let mut publish_time = Utc::now().timestamp();
    let mut is_scheduled = false;
    let limit = state.config.message_size_limit;
    if let Some(d) = delay_str {
        match util::parse_future_time(&d) {
            Ok(t) => {
                let now = Utc::now().timestamp();
                if t > now {
                    let diff = t - now;
                    if diff < state.config.message_delay_min as i64 { return (StatusCode::BAD_REQUEST, "Delay too short").into_response(); }
                    if diff > state.config.message_delay_max as i64 { return (StatusCode::BAD_REQUEST, "Delay too long").into_response(); }
                    publish_time = t;
                    is_scheduled = true;
                }
            },
            Err(_) => return (StatusCode::BAD_REQUEST, "Invalid delay format").into_response(),
        }
    }

    let mut msg = Message {
        id: id.clone(),
        sequence_id: None,
        time: publish_time,
        expires: None,
        event: EVENT_MESSAGE.to_string(),
        topic: topic_id.clone(),
        title,
        message: message_body,
        priority,
        tags,
        click,
        icon,
        actions,
        attachment: None,
        poll_id: None,
        content_type: Some("text/plain".to_string()),
        encoding: None,
        sender: Some(addr.ip().to_string()),
        user: if user.role == Role::Anonymous { None } else { Some(user.id.clone()) },
        published: !is_scheduled,
    };

    let tier_info = user.tier.info();
    let attachment_total_size_limit = if user.role == Role::Anonymous { state.config.visitor_attachment_total_size_limit as i64 } else { tier_info.attachment_total_size_limit };
    let attachment_expiry_duration_secs = if user.role == Role::Anonymous { state.config.attachment_expiry_duration } else { tier_info.attachment_expiry_duration as u64 };
    let attachment_expiry = Utc::now().timestamp() + attachment_expiry_duration_secs as i64;
    msg.expires = Some(attachment_expiry);

    let mut attachment = None;
    if filename.is_some() || attachment_url.is_some() {
        if state.file_storage.get_total_size().await >= state.config.total_attachment_size_limit as u64 {
            return (StatusCode::INSUFFICIENT_STORAGE, "Global storage limit reached").into_response();
        }
        let current_user_size = if user.role == Role::Anonymous { state.store.get_sender_attachment_size(&addr.ip().to_string()).await.unwrap_or(0) } else { state.store.get_user_attachment_size(&user.id).await.unwrap_or(0) };
        if current_user_size >= attachment_total_size_limit {
            return (StatusCode::TOO_MANY_REQUESTS, "User storage limit reached").into_response();
        }
    }

    if json_request.is_none() {
        if let Some(ref fname) = filename {
            let body = body_opt.take().unwrap_or_else(|| Body::from(bytes.clone().unwrap()));
            if !state.limiter.check_bandwidth(addr.ip(), 0) { return (StatusCode::TOO_MANY_REQUESTS, "Daily bandwidth limit reached").into_response(); }
            let stream = http_body_util::BodyStream::new(body).map(|r| r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))).map(|r| r.and_then(|f| Ok(f.into_data().unwrap_or_default())));
            let reader = tokio_util::io::StreamReader::new(stream);
            let size = match state.file_storage.write(&id, reader).await {
                Ok(s) => s,
                Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to write attachment").into_response(),
            };
            state.limiter.check_bandwidth(addr.ip(), size as usize);
            let att = Attachment { name: fname.clone(), type_: None, size: Some(size as i64), expires: Some(attachment_expiry), url: format!("{}/file/{}", state.config.base_url, id) };
            if msg.message.is_none() { msg.message = Some(format!("You received a file: {}", att.name)); }
            attachment = Some(att);
        } else if attachment_url.is_none() {
            let body = body_opt.take().unwrap_or_else(|| Body::from(bytes.clone().unwrap()));
            let mut buffer = Vec::new();
            let mut stream = http_body_util::BodyStream::new(body);
            let mut is_attachment = false;
            while let Some(frame_res) = stream.next().await {
                if let Ok(frame) = frame_res {
                    if let Ok(bytes) = frame.into_data() {
                        if buffer.len() + bytes.len() > limit { is_attachment = true; buffer.extend_from_slice(&bytes); break; }
                        buffer.extend_from_slice(&bytes);
                    }
                }
            }
            if !is_attachment {
                if let Ok(s) = String::from_utf8(buffer.clone()) { msg.message = Some(s); } else if !buffer.is_empty() { is_attachment = true; }
            }
            if is_attachment {
                if !state.limiter.check_bandwidth(addr.ip(), buffer.len()) { return (StatusCode::TOO_MANY_REQUESTS, "Daily bandwidth limit reached").into_response(); }
                let buffer_stream = tokio_stream::iter(vec![Ok(axum::body::Bytes::from(buffer))]);
                let rest_stream = stream.map(|r| r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))).map(|r| r.and_then(|f| Ok(f.into_data().unwrap_or_default())));
                let combined_stream = buffer_stream.chain(rest_stream);
                let reader = tokio_util::io::StreamReader::new(combined_stream);
                let size = match state.file_storage.write(&id, reader).await {
                    Ok(s) => s,
                    Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to write attachment").into_response(),
                };
                state.limiter.check_bandwidth(addr.ip(), size as usize);
                let att = Attachment { name: "attachment.bin".to_string(), type_: None, size: Some(size as i64), expires: Some(attachment_expiry), url: format!("{}/file/{}", state.config.base_url, id) };
                if msg.message.is_none() { msg.message = Some(format!("You received a file: {}", att.name)); }
                attachment = Some(att);
            }
        }
    }

    if let Some(url) = attachment_url {
        let att = Attachment { name: filename.unwrap_or_else(|| "attachment".to_string()), type_: None, size: None, expires: None, url };
        if msg.message.is_none() { msg.message = Some(format!("You received a file: {}", att.name)); }
        attachment = Some(att);
    }
    
    msg.attachment = attachment;
    let msg = match state.process_publish(msg, None).await {
        Ok(m) => m,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to process message").into_response(),
    };

    if let Some(email) = email_addr {
        let mailer = state.mailer.clone();
        let msg_clone = msg.clone();
        tokio::spawn(async move { let _ = mailer.send(&email, &msg_clone).await; });
    }

    (StatusCode::OK, Json(msg)).into_response()
}

#[instrument(skip(state))]
async fn handle_subscribe_sse(
    State(state): State<AppState>,
    Path(topic_id): Path<String>,
    auth: Auth,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let user = auth.0;
    match state.auth_manager.authorize(&user, &topic_id, Permission::Read).await {
        Ok(true) => {},
        _ => return (StatusCode::FORBIDDEN, "Forbidden").into_response(),
    }

    let topic = state.get_topic(&topic_id);
    let rx = topic.subscribe();
    let since = params.get("since").map(|s| util::Since::parse(s)).unwrap_or(util::Since::None);
    
    let mut cached_msgs = Vec::new();
    if let util::Since::All | util::Since::Time(_) | util::Since::Latest = since {
        let time = match since { util::Since::All => 0, util::Since::Time(t) => t, _ => 0 };
        if let Ok(msgs) = state.store.get_messages(&topic_id, time).await { cached_msgs = msgs; }
    }
    
    let open_msg = Message { id: Uuid::new_v4().to_string().chars().take(12).collect(), sequence_id: None, time: Utc::now().timestamp(), expires: None, event: EVENT_OPEN.to_string(), topic: topic_id.clone(), title: None, message: None, priority: None, tags: None, click: None, icon: None, actions: None, attachment: None, poll_id: None, content_type: None, encoding: None, sender: None, user: None, published: true };
    let stream = BroadcastStream::new(rx).map(|msg| {
        match msg {
            Ok(m) => { let json = serde_json::to_string(&*m).unwrap_or_default(); Event::default().event(&m.event).data(json) }
            Err(_) => Event::default().event("error").data("lagged"),
        }
    });
    let open_event = Event::default().event(EVENT_OPEN).data(serde_json::to_string(&open_msg).unwrap_or_default());
    let cached_events: Vec<Result<Event, Infallible>> = cached_msgs.into_iter().map(|m| { let json = serde_json::to_string(&m).unwrap_or_default(); Ok(Event::default().event(&m.event).data(json)) }).collect();
    let stream = tokio_stream::iter(vec![Ok(open_event)]).chain(tokio_stream::iter(cached_events)).chain(stream.map(Ok));
    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::new().interval(Duration::from_secs(45)).text("keepalive")).into_response()
}

async fn handle_subscribe_json(state: State<AppState>, path: Path<String>, auth: Auth, params: Query<HashMap<String, String>>) -> impl IntoResponse {
    handle_subscribe_http(state, path, auth, params, "application/x-ndjson", |m| { let mut json = serde_json::to_string(&m).unwrap_or_default(); json.push('\n'); json }).await
}

async fn handle_subscribe_raw(state: State<AppState>, path: Path<String>, auth: Auth, params: Query<HashMap<String, String>>) -> impl IntoResponse {
    handle_subscribe_http(state, path, auth, params, "text/plain", |m| { if m.event == EVENT_MESSAGE { let mut s = m.message.clone().unwrap_or_default().replace('\n', " "); s.push('\n'); s } else { "\n".to_string() } }).await
}

async fn handle_subscribe_http<F>(State(state): State<AppState>, Path(topic_id): Path<String>, auth: Auth, Query(params): Query<HashMap<String, String>>, content_type: &'static str, encoder: F) -> impl IntoResponse 
where F: Fn(&Message) -> String + Send + Sync + 'static 
{
    let user = auth.0;
    match state.auth_manager.authorize(&user, &topic_id, Permission::Read).await {
        Ok(true) => {},
        _ => return (StatusCode::FORBIDDEN, "Forbidden").into_response(),
    }
    let topic = state.get_topic(&topic_id);
    let rx = topic.subscribe();
    let since = params.get("since").map(|s| util::Since::parse(s)).unwrap_or(util::Since::None);
    let poll = params.get("poll").map(|s| s == "1" || s == "yes").unwrap_or(false);
    let mut cached_msgs = Vec::new();
    if let util::Since::All | util::Since::Time(_) | util::Since::Latest = since {
        let time = match since { util::Since::All => 0, util::Since::Time(t) => t, _ => 0 };
        if let Ok(msgs) = state.store.get_messages(&topic_id, time).await { cached_msgs = msgs; }
    }
    let open_msg = Message { id: Uuid::new_v4().to_string().chars().take(12).collect(), sequence_id: None, time: Utc::now().timestamp(), expires: None, event: EVENT_OPEN.to_string(), topic: topic_id.clone(), title: None, message: None, priority: None, tags: None, click: None, icon: None, actions: None, attachment: None, poll_id: None, content_type: None, encoding: None, sender: None, user: None, published: true };
    if poll {
        let mut body_str = String::new();
        for m in cached_msgs { body_str.push_str(&encoder(&m)); }
        return (StatusCode::OK, [("Content-Type", format!("{}; charset=utf-8", content_type))], body_str).into_response();
    }
    let encoder_arc = Arc::new(encoder);
    let encoder_clone = encoder_arc.clone();
    let stream = BroadcastStream::new(rx).map(move |msg| { match msg { Ok(m) => { let s = encoder_clone(&m); Ok(s) as Result<String, Infallible> } Err(_) => Ok("".to_string()) } }).filter(|res: &Result<String, Infallible>| { let keep = match res { Ok(s) => !s.is_empty(), Err(_) => true }; futures::future::ready(keep) });
    let open_str = encoder_arc(&open_msg);
    let cached_strings: Vec<Result<String, Infallible>> = cached_msgs.into_iter().map(move |m| Ok(encoder_arc(&m))).collect();
    let stream = tokio_stream::iter(vec![Ok(open_str)]).chain(tokio_stream::iter(cached_strings)).chain(stream).map(|res: Result<String, Infallible>| res.map(|s| Bytes::from(s)));
    (StatusCode::OK, [("Content-Type", format!("{}; charset=utf-8", content_type))], Body::from_stream(stream)).into_response()
}

#[instrument(skip(state))]
async fn handle_file(State(state): State<AppState>, Path(file_id_ext): Path<String>) -> impl IntoResponse {
    let file_id = file_id_ext.split('.').next().unwrap_or(&file_id_ext);
    if !state.file_storage.exists(file_id) { return StatusCode::NOT_FOUND.into_response(); }
    let path = state.file_storage.get_path(file_id);
    match tokio::fs::File::open(&path).await {
        Ok(file) => { let stream = tokio_util::io::ReaderStream::new(file); let body = Body::from_stream(stream); (StatusCode::OK, body).into_response() },
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}
