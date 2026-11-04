use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tower_http::limit::RequestBodyLimitLayer;

use crate::model::ClientSignal;
use crate::error::BanterError;
use crate::live::Live;
use crate::model::*;
use crate::store::Store;

#[derive(Clone)]
struct HttpState {
    store: Arc<Store>,
    live: Arc<Live>,
}

struct Auth(Account);

impl FromRequestParts<HttpState> for Auth {
    type Rejection = BanterError;

    async fn from_request_parts(parts: &mut Parts, state: &HttpState) -> Result<Self, Self::Rejection> {
        let header_value = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        let token = header_value
            .strip_prefix("Bearer ")
            .ok_or(BanterError::InvalidCredentials)?;
        Ok(Auth(state.store.session_account(token).await?))
    }
}

impl IntoResponse for BanterError {
    fn into_response(self) -> Response {
        let status = match &self {
            BanterError::InvalidCredentials => StatusCode::UNAUTHORIZED,
            BanterError::UsernameTaken
            | BanterError::AlreadyFriends
            | BanterError::RequestExists
            | BanterError::AlreadyMember
            | BanterError::BoardConflict => StatusCode::CONFLICT,
            BanterError::NotFound | BanterError::InvalidInvite => StatusCode::NOT_FOUND,
            BanterError::NotMember
            | BanterError::NotOwner
            | BanterError::RoleHierarchy
            | BanterError::NotAuthor
            | BanterError::NotFriend
            | BanterError::NotPending => StatusCode::FORBIDDEN,
            BanterError::Storage(_) | BanterError::Bind(_) => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::BAD_REQUEST,
        };
        if matches!(status, StatusCode::INTERNAL_SERVER_ERROR) {
            crate::debuglog::record("error", format!("{}: {self}", self.code()));
        }
        let body = Json(json!({"error": self.to_string(), "code": self.code()}));
        (status, body).into_response()
    }
}

pub fn serve(addr: &str, data_dir: &std::path::Path, database_url: &str) -> Result<(), BanterError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| BanterError::Bind(err.to_string()))?;
    runtime.block_on(async move {
        let store = Arc::new(Store::open(data_dir, database_url).await?);
        let live = Arc::new(Live::new());
        let state = HttpState {
            store: Arc::clone(&store),
            live: Arc::clone(&live),
        };
        let app = router(state.clone()).layer(RequestBodyLimitLayer::new(20 * 1024 * 1024));
        let listener = tokio::net::TcpListener::bind(addr).await.map_err(|err| {
            crate::debuglog::record("error", format!("bind failed on {addr}: {err}"));
            BanterError::Bind(err.to_string())
        })?;
        crate::debuglog::record("info", format!("listening on {addr}"));
        let idle_store = Arc::clone(&store);
        let idle_live = Arc::clone(&live);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(15)).await;
                for (user_id, presence) in idle_live.refresh(now_ms()) {
                    let audience = idle_store.related_user_ids(user_id).await.unwrap_or_default();
                    let custom_status = idle_live.custom_status(user_id);
                    idle_live.send_event(
                        &audience,
                        &ServerEvent::Presence {
                            user_id,
                            state: presence,
                            custom_status,
                        },
                    );
                }
            }
        });
        axum::serve(listener, app)
            .await
            .map_err(|err| BanterError::Bind(err.to_string()))
    })
}

fn router(state: HttpState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/debug", get(server_debug))
        .route("/v1/register", post(register))
        .route("/v1/login", post(login))
        .route("/v1/logout", post(logout))
        .route("/v1/me", get(me).patch(patch_me))
        .route("/v1/me/avatar", post(upload_avatar).delete(delete_avatar))
        .route("/v1/me/profile", axum::routing::patch(patch_profile))
        .route("/v1/me/banner", post(upload_banner).delete(delete_banner))
        .route("/v1/profile/cosmetics", get(list_cosmetics))
        .route("/v1/profile/badges", get(list_badges))
        .route("/v1/users/{id}", get(user_profile))
        .route("/v1/users/{id}/avatar", get(avatar))
        .route("/v1/users/{id}/banner", get(banner))
        .route("/v1/friends/requests", get(list_requests).post(create_request))
        .route("/v1/friends/requests/{id}/accept", post(accept_request))
        .route("/v1/friends/requests/{id}/decline", post(decline_request))
        .route("/v1/friends", get(list_friends))
        .route("/v1/presence", get(list_presence))
        .route("/v1/servers", get(list_servers).post(create_server))
        .route("/v1/servers/{id}/invites", post(create_invite))
        .route("/v1/invites/{code}/join", post(join_invite))
        .route("/v1/servers/{id}/members", get(members))
        .route("/v1/servers/{id}/members/{user_id}/roles", axum::routing::put(set_member_roles))
        .route("/v1/servers/{id}/roles", get(list_roles).post(create_role))
        .route(
            "/v1/servers/{id}/roles/{role_id}",
            axum::routing::patch(update_role).delete(delete_role),
        )
        .route("/v1/servers/{id}/nickname", axum::routing::put(set_nickname))
        .route("/v1/servers/{id}/categories", post(create_category))
        .route("/v1/servers/{id}/channels", post(create_channel))
        .route(
            "/v1/servers/{id}/channels/{channel_id}",
            axum::routing::patch(update_channel),
        )
        .route("/v1/servers/{id}/layout", get(layout))
        .route("/v1/channels/{id}/messages", get(channel_messages))
        .route("/v1/channels/{id}/board", get(get_board))
        .route("/v1/channels/{id}/board/ops", post(apply_board_ops))
        .route("/v1/channels/{id}/board/assets", post(upload_board_asset))
        .route("/v1/board-assets/{id}", get(board_asset))
        .route("/v1/dms", get(list_dms).post(open_dm))
        .route("/v1/dms/{id}/messages", get(dm_messages))
        .route("/v1/dms/{id}/call", post(start_call))
        .route("/v1/dms/{id}/call/accept", post(accept_call))
        .route("/v1/dms/{id}/call/decline", post(decline_call))
        .route("/v1/dms/{id}/call/end", post(end_call))
        .route("/v1/messages", post(send_message))
        .route("/v1/messages/{id}", axum::routing::patch(edit_message).delete(delete_message))
        .route("/v1/attachments/{id}", get(attachment))
        .route("/v1/mentions", get(mentions))
        .route("/v1/mentions/{id}/read", post(read_mention))
        .route("/v1/voice/join", post(join_voice))
        .route("/v1/voice/leave", post(leave_voice))
        .route("/v1/voice/flags", post(set_flags))
        .route("/v1/ws", get(ws))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({"ok": true}))
}

async fn server_debug(State(state): State<HttpState>, Auth(_account): Auth) -> Result<Json<ServerDebug>, BanterError> {
    let counts = state.live.counts();
    let (users, servers, channels, messages) = state.store.table_counts().await?;
    let (pool_size, pool_idle) = state.store.pool_usage();
    let (media_slots, media_forwards) = crate::room::media_snapshot();
    Ok(Json(ServerDebug {
        uptime_ms: crate::debuglog::uptime_ms(),
        rss_bytes: crate::debuglog::rss_bytes(),
        online: counts.online,
        idle: counts.idle,
        sockets: counts.sockets,
        voice_members: counts.voice_members,
        voice_channels: counts.voice_channels,
        calls: counts.calls,
        media_slots,
        media_forwards,
        users,
        servers,
        channels,
        messages,
        pool_size,
        pool_idle,
        logs: crate::debuglog::recent(),
    }))
}

#[derive(Deserialize)]
struct RegisterBody {
    username: String,
    display_name: String,
    password: String,
}

async fn register(State(state): State<HttpState>, Json(body): Json<RegisterBody>) -> Result<Json<Session>, BanterError> {
    let account = state.store.register(&body.username, &body.display_name, &body.password).await?;
    let session = state.store.login(&account.username, &body.password).await?;
    crate::debuglog::record("info", format!("registered {}", account.username));
    Ok(Json(session))
}

#[derive(Deserialize)]
struct LoginBody {
    username: String,
    password: String,
}

async fn login(State(state): State<HttpState>, Json(body): Json<LoginBody>) -> Result<Json<Session>, BanterError> {
    match state.store.login(&body.username, &body.password).await {
        Ok(session) => {
            crate::debuglog::record("info", format!("login {}", session.account.username));
            Ok(Json(session))
        }
        Err(err) => {
            crate::debuglog::record("warn", format!("login failed for {}", body.username.trim()));
            Err(err)
        }
    }
}

async fn logout(State(state): State<HttpState>, headers: HeaderMap) -> Result<StatusCode, BanterError> {
    if let Some(token) = bearer(&headers) {
        state.store.logout(token).await?;
        crate::debuglog::record("info", "logout");
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn me(Auth(account): Auth) -> Json<Account> {
    Json(account)
}

#[derive(Deserialize)]
struct DisplayNameBody {
    display_name: String,
}

async fn patch_me(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Json(body): Json<DisplayNameBody>,
) -> Result<Json<Account>, BanterError> {
    Ok(Json(state.store.set_display_name(account.id, &body.display_name).await?))
}

async fn upload_avatar(
    State(state): State<HttpState>,
    Auth(account): Auth,
    body: Bytes,
) -> Result<StatusCode, BanterError> {
    state.store.set_avatar(account.id, &body).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_avatar(State(state): State<HttpState>, Auth(account): Auth) -> Result<StatusCode, BanterError> {
    state.store.clear_avatar(account.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ProfileQuery {
    server_id: Option<i64>,
}

async fn user_profile(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
    Query(query): Query<ProfileQuery>,
) -> Result<Json<UserProfile>, BanterError> {
    Ok(Json(state.store.user_profile(account.id, id, query.server_id).await?))
}

async fn patch_profile(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Json(body): Json<ProfilePatch>,
) -> Result<Json<UserProfile>, BanterError> {
    let status_patched = body.custom_status.is_some();
    let profile = state.store.update_profile(account.id, body).await?;
    if status_patched {
        state
            .live
            .set_custom_status(account.id, profile.custom_status.clone());
        if let Ok(audience) = state.store.related_user_ids(account.id).await {
            let snap = state.live.presence_snapshot(account.id);
            state.live.send_event(
                &audience,
                &ServerEvent::Presence {
                    user_id: account.id,
                    state: snap.state,
                    custom_status: snap.custom_status,
                },
            );
        }
    }
    Ok(Json(profile))
}

async fn list_cosmetics() -> Json<Vec<crate::CosmeticItem>> {
    Json(crate::cosmetics_catalog())
}

async fn list_badges() -> Json<Vec<crate::BadgeItem>> {
    Json(crate::badges_catalog())
}

async fn upload_banner(
    State(state): State<HttpState>,
    Auth(account): Auth,
    body: Bytes,
) -> Result<StatusCode, BanterError> {
    state.store.set_banner(account.id, &body).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_banner(State(state): State<HttpState>, Auth(account): Auth) -> Result<StatusCode, BanterError> {
    state.store.clear_banner(account.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn avatar(State(state): State<HttpState>, Path(id): Path<i64>) -> Result<Response, BanterError> {
    let bytes = state.store.avatar_bytes(id).await?;
    Ok(([(header::CONTENT_TYPE, "image/png")], bytes).into_response())
}

async fn banner(State(state): State<HttpState>, Path(id): Path<i64>) -> Result<Response, BanterError> {
    let bytes = state.store.banner_bytes(id).await?;
    Ok(([(header::CONTENT_TYPE, "image/png")], bytes).into_response())
}

#[derive(Deserialize)]
struct UsernameBody {
    username: String,
}

async fn create_request(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Json(body): Json<UsernameBody>,
) -> Result<Json<FriendRequest>, BanterError> {
    let request = state.store.send_friend_request(account.id, &body.username).await?;
    state.live.send_event(
        &[request.to_id, request.from_id],
        &ServerEvent::FriendRequest {
            request: request.clone(),
        },
    );
    if state.store.friends(account.id).await?.iter().any(|friend| friend.id == request.to_id) {
        state.live.send_event(&[request.from_id, request.to_id], &ServerEvent::FriendsChanged);
    }
    Ok(Json(request))
}

async fn list_requests(
    State(state): State<HttpState>,
    Auth(account): Auth,
) -> Result<Json<serde_json::Value>, BanterError> {
    Ok(Json(json!({
        "incoming": state.store.incoming_requests(account.id).await?,
        "outgoing": state.store.outgoing_requests(account.id).await?,
    })))
}

async fn accept_request(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<StatusCode, BanterError> {
    let incoming = state.store.incoming_requests(account.id).await?;
    state.store.accept_friend_request(account.id, id).await?;
    let mut ids = vec![account.id];
    if let Some(request) = incoming.iter().find(|request| request.id == id) {
        ids.push(request.from_id);
    }
    state.live.send_event(&ids, &ServerEvent::FriendsChanged);
    Ok(StatusCode::NO_CONTENT)
}

async fn decline_request(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<StatusCode, BanterError> {
    state.store.decline_friend_request(account.id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_friends(State(state): State<HttpState>, Auth(account): Auth) -> Result<Json<Vec<Friend>>, BanterError> {
    Ok(Json(state.store.friends(account.id).await?))
}

async fn list_presence(
    State(state): State<HttpState>,
    Auth(account): Auth,
) -> Result<Json<Vec<serde_json::Value>>, BanterError> {
    let ids = state.store.related_user_ids(account.id).await?;
    let presence = ids
        .into_iter()
        .map(|user_id| {
            let snap = state.live.presence_snapshot(user_id);
            json!({
                "user_id": user_id,
                "state": snap.state,
                "custom_status": snap.custom_status,
            })
        })
        .collect();
    Ok(Json(presence))
}

#[derive(Deserialize)]
struct NameBody {
    name: String,
}

async fn create_server(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Json(body): Json<NameBody>,
) -> Result<Json<ServerHome>, BanterError> {
    Ok(Json(state.store.create_server(account.id, &body.name).await?))
}

async fn list_servers(
    State(state): State<HttpState>,
    Auth(account): Auth,
) -> Result<Json<Vec<ServerSummary>>, BanterError> {
    Ok(Json(state.store.servers(account.id).await?))
}

async fn create_invite(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, BanterError> {
    let code = state.store.create_invite(account.id, id).await?;
    Ok(Json(json!({"code": code})))
}

async fn join_invite(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(code): Path<String>,
) -> Result<Json<ServerHome>, BanterError> {
    Ok(Json(state.store.join_invite(account.id, &code).await?))
}

async fn members(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<Json<Vec<serde_json::Value>>, BanterError> {
    let members = state.store.members(account.id, id).await?;
    let enriched = members
        .into_iter()
        .map(|member| {
            json!({
                "user_id": member.user_id,
                "username": member.username,
                "display_name": member.display_name,
                "nickname": member.nickname,
                "is_owner": member.is_owner,
                "has_avatar": member.has_avatar,
                "avatar_rev": member.avatar_rev,
                "presence": state.live.presence(member.user_id),
                "custom_status": state.live.custom_status(member.user_id),
                "roles": member.roles,
            })
        })
        .collect();
    Ok(Json(enriched))
}

#[derive(Deserialize)]
struct NicknameBody {
    nickname: Option<String>,
}

async fn set_nickname(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
    Json(body): Json<NicknameBody>,
) -> Result<StatusCode, BanterError> {
    state
        .store
        .set_nickname(account.id, id, body.nickname.as_deref()).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_roles(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<Json<Vec<GuildRole>>, BanterError> {
    Ok(Json(state.store.roles(account.id, id).await?))
}

#[derive(Deserialize)]
struct RoleBody {
    name: String,
    #[serde(default)]
    color: i64,
    #[serde(default)]
    hoist: bool,
    #[serde(default)]
    permissions: i64,
}

async fn create_role(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
    Json(body): Json<RoleBody>,
) -> Result<Json<GuildRole>, BanterError> {
    Ok(Json(
        state
            .store
            .create_role(account.id, id, &body.name, body.color, body.hoist, body.permissions)
            .await?,
    ))
}

#[derive(Deserialize)]
struct ServerRolePath {
    id: i64,
    role_id: i64,
}

async fn update_role(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(path): Path<ServerRolePath>,
    Json(body): Json<RoleBody>,
) -> Result<Json<GuildRole>, BanterError> {
    Ok(Json(
        state
            .store
            .update_role(
                account.id,
                path.id,
                path.role_id,
                &body.name,
                body.color,
                body.hoist,
                body.permissions,
            )
            .await?,
    ))
}

async fn delete_role(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(path): Path<ServerRolePath>,
) -> Result<StatusCode, BanterError> {
    state.store.delete_role(account.id, path.id, path.role_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct MemberRolePath {
    id: i64,
    user_id: i64,
}

#[derive(Deserialize)]
struct RoleIdsBody {
    role_ids: Vec<i64>,
}

async fn set_member_roles(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(path): Path<MemberRolePath>,
    Json(body): Json<RoleIdsBody>,
) -> Result<StatusCode, BanterError> {
    state
        .store
        .set_member_roles(account.id, path.id, path.user_id, &body.role_ids)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create_category(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
    Json(body): Json<NameBody>,
) -> Result<Json<Category>, BanterError> {
    Ok(Json(state.store.create_category(account.id, id, &body.name).await?))
}

#[derive(Deserialize)]
struct ChannelBody {
    category_id: i64,
    name: String,
    kind: ChannelKind,
    #[serde(default)]
    icon: Option<String>,
}

async fn create_channel(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
    Json(body): Json<ChannelBody>,
) -> Result<Json<Channel>, BanterError> {
    Ok(Json(
        state
            .store
            .create_channel(
                account.id,
                id,
                body.category_id,
                &body.name,
                body.kind,
                body.icon.as_deref(),
            )
            .await?,
    ))
}

#[derive(Deserialize)]
struct ServerChannelPath {
    id: i64,
    channel_id: i64,
}

#[derive(Deserialize)]
struct UpdateChannelBody {
    name: String,
    #[serde(default)]
    icon: Option<String>,
}

async fn update_channel(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(path): Path<ServerChannelPath>,
    Json(body): Json<UpdateChannelBody>,
) -> Result<Json<Channel>, BanterError> {
    Ok(Json(
        state
            .store
            .update_channel(
                account.id,
                path.id,
                path.channel_id,
                &body.name,
                body.icon.as_deref(),
            )
            .await?,
    ))
}

async fn layout(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<Json<Vec<CategoryLayout>>, BanterError> {
    Ok(Json(state.store.layout(account.id, id).await?))
}

#[derive(Deserialize)]
struct ListQuery {
    limit: Option<u32>,
    q: Option<String>,
}

async fn get_board(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<Json<BoardScene>, BanterError> {
    Ok(Json(state.store.board_scene(account.id, id).await?))
}

#[derive(Deserialize)]
struct BoardOpsBody {
    ops: Vec<BoardOp>,
}

async fn apply_board_ops(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
    Json(body): Json<BoardOpsBody>,
) -> Result<Json<BoardScene>, BanterError> {
    if body.ops.is_empty() {
        return Ok(Json(state.store.board_scene(account.id, id).await?));
    }
    let mut scene = None;
    let mut applied = Vec::new();
    for op in body.ops {
        let next = state.store.apply_board_op(account.id, id, op.clone()).await?;
        applied.push(op);
        scene = Some(next);
    }
    let scene = scene.unwrap();
    broadcast_board(&state, id, account.id, scene.revision, applied).await;
    Ok(Json(scene))
}

#[derive(Deserialize)]
struct BoardAssetBody {
    filename: String,
    content_type: Option<String>,
    data_base64: String,
}

async fn upload_board_asset(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
    Json(body): Json<BoardAssetBody>,
) -> Result<Json<BoardAsset>, BanterError> {
    let bytes = STANDARD
        .decode(body.data_base64.trim())
        .map_err(|_| BanterError::FileTooLarge)?;
    let content_type = body.content_type.unwrap_or_else(|| "application/octet-stream".into());
    Ok(Json(state.store.upload_board_asset(
        account.id,
        id,
        &body.filename,
        &content_type,
        &bytes,
    ).await?))
}

async fn board_asset(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<Response, BanterError> {
    let (asset, bytes) = state.store.read_board_asset(account.id, id).await?;
    let filename = asset.filename.replace('"', "");
    Ok((
        [
            (header::CONTENT_TYPE, asset.content_type),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{filename}\"")),
        ],
        bytes,
    )
        .into_response())
}

async fn broadcast_board(state: &HttpState, channel_id: i64, from: i64, revision: i64, ops: Vec<BoardOp>) {
    if let Ok(ids) = state.store.channel_member_ids(channel_id).await {
        state.live.send_event(
            &ids,
            &ServerEvent::BoardChanged {
                channel_id,
                revision,
                ops,
                from,
            },
        );
    }
}

async fn channel_messages(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<Message>>, BanterError> {
    messages_for(&state, account.id, ChatTarget::Channel(id), query).await
}

async fn dm_messages(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<Message>>, BanterError> {
    messages_for(&state, account.id, ChatTarget::Direct(id), query).await
}

async fn messages_for(
    state: &HttpState,
    user_id: i64,
    target: ChatTarget,
    query: ListQuery,
) -> Result<Json<Vec<Message>>, BanterError> {
    let limit = query.limit.unwrap_or(100);
    let messages = match query.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        Some(q) => state.store.search_messages(user_id, target, q).await?,
        None => state.store.messages(user_id, target, limit).await?,
    };
    Ok(Json(messages))
}

#[derive(Deserialize)]
struct OpenDmBody {
    user_id: Option<i64>,
    username: Option<String>,
}

async fn open_dm(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Json(body): Json<OpenDmBody>,
) -> Result<Json<DirectChat>, BanterError> {
    let peer = if let Some(user_id) = body.user_id {
        user_id
    } else if let Some(username) = body.username {
        state.store.user_by_username(&username).await?.id
    } else {
        return Err(BanterError::NotFound);
    };
    Ok(Json(state.store.open_dm(account.id, peer).await?))
}

async fn list_dms(State(state): State<HttpState>, Auth(account): Auth) -> Result<Json<Vec<DirectChat>>, BanterError> {
    Ok(Json(state.store.direct_messages(account.id).await?))
}

#[derive(Deserialize)]
struct IncomingFile {
    filename: String,
    data_base64: String,
}

#[derive(Deserialize)]
struct SendBody {
    target: ChatTarget,
    body: String,
    reply_to: Option<i64>,
    #[serde(default)]
    files: Vec<IncomingFile>,
}

async fn send_message(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Json(body): Json<SendBody>,
) -> Result<Json<Message>, BanterError> {
    let mut files = Vec::new();
    for file in &body.files {
        let bytes = STANDARD
            .decode(file.data_base64.trim())
            .map_err(|_| BanterError::FileTooLarge)?;
        files.push((file.filename.clone(), bytes));
    }
    let borrowed: Vec<(&str, &[u8])> = files.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice())).collect();
    let message = state
        .store
        .send_message(account.id, body.target, &body.body, body.reply_to, &borrowed).await?;
    let ids = state.store.target_user_ids(body.target).await?;
    state.live.send_event(
        &ids,
        &ServerEvent::MessageCreated {
            target: body.target,
            message: message.clone(),
        },
    );
    Ok(Json(message))
}

#[derive(Deserialize)]
struct EditBody {
    body: String,
}

async fn edit_message(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
    Json(body): Json<EditBody>,
) -> Result<Json<Message>, BanterError> {
    let message = state.store.edit_message(account.id, id, &body.body).await?;
    let target = state.store.message_target(id).await?;
    let ids = state.store.target_user_ids(target).await?;
    state.live.send_event(
        &ids,
        &ServerEvent::MessageEdited {
            target,
            message: message.clone(),
        },
    );
    Ok(Json(message))
}

async fn delete_message(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<StatusCode, BanterError> {
    let target = state.store.message_target(id).await?;
    let ids = state.store.target_user_ids(target).await?;
    state.store.delete_message(account.id, id).await?;
    state.live.send_event(&ids, &ServerEvent::MessageDeleted { target, message_id: id });
    Ok(StatusCode::NO_CONTENT)
}

async fn attachment(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<Response, BanterError> {
    let (filename, bytes) = state.store.read_attachment(account.id, id).await?;
    let filename = filename.replace('"', "");
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{filename}\"")),
        ],
        bytes,
    )
        .into_response())
}

async fn mentions(State(state): State<HttpState>, Auth(account): Auth) -> Result<Json<Vec<Mention>>, BanterError> {
    Ok(Json(state.store.unread_mentions(account.id).await?))
}

async fn read_mention(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<StatusCode, BanterError> {
    state.store.mark_mention_read(account.id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct VoiceJoinBody {
    channel_id: i64,
}

async fn join_voice(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Json(body): Json<VoiceJoinBody>,
) -> Result<Json<Vec<VoiceMember>>, BanterError> {
    state.live.join_server_voice(&state.store, account.id, body.channel_id).await?;
    crate::debuglog::record("info", format!("voice join user {} channel {}", account.id, body.channel_id));
    Ok(Json(state.live.voice_channel_members(body.channel_id)))
}

async fn leave_voice(State(state): State<HttpState>, Auth(account): Auth) -> StatusCode {
    state.live.leave_voice(account.id);
    crate::debuglog::record("info", format!("voice leave user {}", account.id));
    StatusCode::NO_CONTENT
}

async fn set_flags(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Json(flags): Json<VoiceFlags>,
) -> StatusCode {
    state.live.set_flags(account.id, flags);
    StatusCode::NO_CONTENT
}

async fn start_call(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<Json<CallSnapshot>, BanterError> {
    let call = state.live.start_dm_call(&state.store, account.id, id).await?;
    crate::debuglog::record("info", format!("call start user {} dm {id}", account.id));
    Ok(Json(call))
}

async fn accept_call(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<Json<CallSnapshot>, BanterError> {
    let call = state.live.accept_dm_call(&state.store, account.id, id).await?;
    crate::debuglog::record("info", format!("call accept user {} dm {id}", account.id));
    Ok(Json(call))
}

async fn decline_call(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<StatusCode, BanterError> {
    state.live.decline_dm_call(account.id, id)?;
    crate::debuglog::record("info", format!("call decline user {} dm {id}", account.id));
    Ok(StatusCode::NO_CONTENT)
}

async fn end_call(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<StatusCode, BanterError> {
    state.live.end_dm_call(account.id, id)?;
    crate::debuglog::record("info", format!("call end user {} dm {id}", account.id));
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct TokenQuery {
    token: String,
}

async fn ws(
    State(state): State<HttpState>,
    Query(query): Query<TokenQuery>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, BanterError> {
    let account = state.store.session_account(&query.token).await?;
    crate::debuglog::record("info", format!("websocket connected user {}", account.id));
    Ok(upgrade.on_upgrade(move |socket| handle_socket(state, account.id, socket)))
}

async fn handle_socket(state: HttpState, user_id: i64, socket: WebSocket) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let socket_id = state.live.register_socket(user_id, tx);
    let custom_status = state.store.custom_status(user_id).await.ok().flatten();
    state.live.connect(user_id, now_ms(), custom_status.clone());
    if let Ok(audience) = state.store.related_user_ids(user_id).await {
        state.live.send_event(
            &audience,
            &ServerEvent::Presence {
                user_id,
                state: Presence::Online,
                custom_status,
            },
        );
    }
    let (mut sink, mut stream) = socket.split();
    let writer = tokio::spawn(async move {
        while let Some(outbound) = rx.recv().await {
            let crate::live::Outbound::Text(text) = outbound;
            let message = WsMessage::Text(text.into());
            if sink.send(message).await.is_err() {
                break;
            }
        }
    });
    while let Some(incoming) = stream.next().await {
        let Ok(message) = incoming else { break };
        match message {
            WsMessage::Text(text) => {
                if let Ok(signal) = serde_json::from_str::<ClientSignal>(&text) {
                    if matches!(signal, ClientSignal::Activity) {
                        note_ws_activity(&state, user_id).await;
                    }
                    if let ClientSignal::BoardOp { channel_id, op } = &signal {
                        apply_board_signal(&state, user_id, *channel_id, op.clone()).await;
                    }
                    let signal_for_room = signal.clone();
                    if state.live.handle_signal(user_id, signal).is_ok() {
                        apply_voice_signal(&state, user_id, signal_for_room);
                    }
                } else if text.contains("activity") {
                    note_ws_activity(&state, user_id).await;
                }
            }
            WsMessage::Binary(_) => {
                // Media is UDP. Binary websocket frames are not a media path.
            }
            WsMessage::Close(_) => break,
            _ => {}
        }
    }
    writer.abort();
    state.live.unregister_socket(user_id, socket_id);
    state.live.disconnect(user_id);
    crate::debuglog::record("info", format!("websocket disconnected user {user_id}"));
    if let Ok(audience) = state.store.related_user_ids(user_id).await {
        state.live.send_event(
            &audience,
            &ServerEvent::Presence {
                user_id,
                state: Presence::Offline,
                custom_status: None,
            },
        );
    }
}

fn apply_voice_signal(state: &HttpState, user_id: i64, signal: ClientSignal) {
    match signal {
        ClientSignal::RtcOffer {
            place: MediaPlace::VoiceChannel(channel_id),
            sdp,
        } => crate::room::accept_offer(Arc::clone(&state.live), user_id, channel_id, sdp),
        ClientSignal::RtcAnswer {
            place: MediaPlace::VoiceChannel(_),
            sdp,
        } => crate::room::accept_answer(user_id, sdp),
        ClientSignal::RtcIce {
            place: MediaPlace::VoiceChannel(_),
            candidate,
            sdp_mid,
            sdp_mline_index,
        } => crate::room::accept_ice(user_id, candidate, sdp_mid, sdp_mline_index),
        ClientSignal::SubscribeLayer { .. } => {
            for request in state.live.take_keyframes() {
                crate::room::request_pli(request.publisher, request.layer);
            }
        }
        _ => {}
    }
}

async fn apply_board_signal(state: &HttpState, user_id: i64, channel_id: i64, op: BoardOp) {
    match state.store.apply_board_op(user_id, channel_id, op.clone()).await {
        Ok(scene) => {
            broadcast_board(state, channel_id, user_id, scene.revision, vec![op]).await;
        }
        Err(_) => {}
    }
}

async fn note_ws_activity(state: &HttpState, user_id: i64) {
    state.live.note_activity(user_id, now_ms());
    if let Ok(audience) = state.store.related_user_ids(user_id).await {
        let snap = state.live.presence_snapshot(user_id);
        state.live.send_event(
            &audience,
            &ServerEvent::Presence {
                user_id,
                state: snap.state,
                custom_status: snap.custom_status,
            },
        );
    }
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}
