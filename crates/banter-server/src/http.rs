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
        Ok(Auth(state.store.session_account(token)?))
    }
}

impl IntoResponse for BanterError {
    fn into_response(self) -> Response {
        let status = match &self {
            BanterError::InvalidCredentials => StatusCode::UNAUTHORIZED,
            BanterError::UsernameTaken
            | BanterError::AlreadyFriends
            | BanterError::RequestExists
            | BanterError::AlreadyMember => StatusCode::CONFLICT,
            BanterError::NotFound | BanterError::InvalidInvite => StatusCode::NOT_FOUND,
            BanterError::NotMember
            | BanterError::NotOwner
            | BanterError::NotAuthor
            | BanterError::NotFriend
            | BanterError::NotPending => StatusCode::FORBIDDEN,
            BanterError::Storage(_) | BanterError::Bind(_) => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::BAD_REQUEST,
        };
        let body = Json(json!({"error": self.to_string(), "code": self.code()}));
        (status, body).into_response()
    }
}

pub fn serve(addr: &str, data_dir: &std::path::Path) -> Result<(), BanterError> {
    let store = Arc::new(Store::open(data_dir)?);
    let live = Arc::new(Live::new());
    let state = HttpState {
        store: Arc::clone(&store),
        live: Arc::clone(&live),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|err| BanterError::Bind(err.to_string()))?;
    runtime.block_on(async move {
        let app = router(state.clone()).layer(RequestBodyLimitLayer::new(20 * 1024 * 1024));
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|err| BanterError::Bind(err.to_string()))?;
        let idle_store = Arc::clone(&store);
        let idle_live = Arc::clone(&live);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(15)).await;
                for (user_id, presence) in idle_live.refresh(now_ms()) {
                    let audience = idle_store.related_user_ids(user_id).unwrap_or_default();
                    idle_live.send_event(
                        &audience,
                        &ServerEvent::Presence {
                            user_id,
                            state: presence,
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
        .route("/v1/register", post(register))
        .route("/v1/login", post(login))
        .route("/v1/logout", post(logout))
        .route("/v1/me", get(me).patch(patch_me))
        .route("/v1/me/avatar", post(upload_avatar).delete(delete_avatar))
        .route("/v1/users/{id}/avatar", get(avatar))
        .route("/v1/friends/requests", get(list_requests).post(create_request))
        .route("/v1/friends/requests/{id}/accept", post(accept_request))
        .route("/v1/friends/requests/{id}/decline", post(decline_request))
        .route("/v1/friends", get(list_friends))
        .route("/v1/presence", get(list_presence))
        .route("/v1/servers", get(list_servers).post(create_server))
        .route("/v1/servers/{id}/invites", post(create_invite))
        .route("/v1/invites/{code}/join", post(join_invite))
        .route("/v1/servers/{id}/members", get(members))
        .route("/v1/servers/{id}/nickname", axum::routing::put(set_nickname))
        .route("/v1/servers/{id}/categories", post(create_category))
        .route("/v1/servers/{id}/channels", post(create_channel))
        .route("/v1/servers/{id}/layout", get(layout))
        .route("/v1/channels/{id}/messages", get(channel_messages))
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

#[derive(Deserialize)]
struct RegisterBody {
    username: String,
    display_name: String,
    password: String,
}

async fn register(State(state): State<HttpState>, Json(body): Json<RegisterBody>) -> Result<Json<Session>, BanterError> {
    let account = state.store.register(&body.username, &body.display_name, &body.password)?;
    let session = state.store.login(&account.username, &body.password)?;
    Ok(Json(session))
}

#[derive(Deserialize)]
struct LoginBody {
    username: String,
    password: String,
}

async fn login(State(state): State<HttpState>, Json(body): Json<LoginBody>) -> Result<Json<Session>, BanterError> {
    Ok(Json(state.store.login(&body.username, &body.password)?))
}

async fn logout(State(state): State<HttpState>, headers: HeaderMap) -> Result<StatusCode, BanterError> {
    if let Some(token) = bearer(&headers) {
        state.store.logout(token)?;
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
    Ok(Json(state.store.set_display_name(account.id, &body.display_name)?))
}

async fn upload_avatar(
    State(state): State<HttpState>,
    Auth(account): Auth,
    body: Bytes,
) -> Result<StatusCode, BanterError> {
    state.store.set_avatar(account.id, &body)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_avatar(State(state): State<HttpState>, Auth(account): Auth) -> Result<StatusCode, BanterError> {
    state.store.clear_avatar(account.id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn avatar(State(state): State<HttpState>, Path(id): Path<i64>) -> Result<Response, BanterError> {
    let bytes = state.store.avatar_bytes(id)?;
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
    let request = state.store.send_friend_request(account.id, &body.username)?;
    state.live.send_event(
        &[request.to_id, request.from_id],
        &ServerEvent::FriendRequest {
            request: request.clone(),
        },
    );
    if state.store.friends(account.id)?.iter().any(|friend| friend.id == request.to_id) {
        state.live.send_event(&[request.from_id, request.to_id], &ServerEvent::FriendsChanged);
    }
    Ok(Json(request))
}

async fn list_requests(
    State(state): State<HttpState>,
    Auth(account): Auth,
) -> Result<Json<serde_json::Value>, BanterError> {
    Ok(Json(json!({
        "incoming": state.store.incoming_requests(account.id)?,
        "outgoing": state.store.outgoing_requests(account.id)?,
    })))
}

async fn accept_request(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<StatusCode, BanterError> {
    let incoming = state.store.incoming_requests(account.id)?;
    state.store.accept_friend_request(account.id, id)?;
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
    state.store.decline_friend_request(account.id, id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_friends(State(state): State<HttpState>, Auth(account): Auth) -> Result<Json<Vec<Friend>>, BanterError> {
    Ok(Json(state.store.friends(account.id)?))
}

async fn list_presence(
    State(state): State<HttpState>,
    Auth(account): Auth,
) -> Result<Json<Vec<serde_json::Value>>, BanterError> {
    let ids = state.store.related_user_ids(account.id)?;
    let presence = ids
        .into_iter()
        .map(|user_id| {
            json!({
                "user_id": user_id,
                "state": state.live.presence(user_id),
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
    Ok(Json(state.store.create_server(account.id, &body.name)?))
}

async fn list_servers(
    State(state): State<HttpState>,
    Auth(account): Auth,
) -> Result<Json<Vec<ServerSummary>>, BanterError> {
    Ok(Json(state.store.servers(account.id)?))
}

async fn create_invite(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, BanterError> {
    let code = state.store.create_invite(account.id, id)?;
    Ok(Json(json!({"code": code})))
}

async fn join_invite(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(code): Path<String>,
) -> Result<Json<ServerHome>, BanterError> {
    Ok(Json(state.store.join_invite(account.id, &code)?))
}

async fn members(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<Json<Vec<serde_json::Value>>, BanterError> {
    let members = state.store.members(account.id, id)?;
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
        .set_nickname(account.id, id, body.nickname.as_deref())?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create_category(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
    Json(body): Json<NameBody>,
) -> Result<Json<Category>, BanterError> {
    Ok(Json(state.store.create_category(account.id, id, &body.name)?))
}

#[derive(Deserialize)]
struct ChannelBody {
    category_id: i64,
    name: String,
    kind: ChannelKind,
}

async fn create_channel(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
    Json(body): Json<ChannelBody>,
) -> Result<Json<Channel>, BanterError> {
    Ok(Json(state.store.create_channel(account.id, id, body.category_id, &body.name, body.kind)?))
}

async fn layout(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<Json<Vec<CategoryLayout>>, BanterError> {
    Ok(Json(state.store.layout(account.id, id)?))
}

#[derive(Deserialize)]
struct ListQuery {
    limit: Option<u32>,
    q: Option<String>,
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
        Some(q) => state.store.search_messages(user_id, target, q)?,
        None => state.store.messages(user_id, target, limit)?,
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
        state.store.user_by_username(&username)?.id
    } else {
        return Err(BanterError::NotFound);
    };
    Ok(Json(state.store.open_dm(account.id, peer)?))
}

async fn list_dms(State(state): State<HttpState>, Auth(account): Auth) -> Result<Json<Vec<DirectChat>>, BanterError> {
    Ok(Json(state.store.direct_messages(account.id)?))
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
        .send_message(account.id, body.target, &body.body, body.reply_to, &borrowed)?;
    let ids = state.store.target_user_ids(body.target)?;
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
    let message = state.store.edit_message(account.id, id, &body.body)?;
    let target = state.store.message_target(id)?;
    let ids = state.store.target_user_ids(target)?;
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
    let target = state.store.message_target(id)?;
    let ids = state.store.target_user_ids(target)?;
    state.store.delete_message(account.id, id)?;
    state.live.send_event(&ids, &ServerEvent::MessageDeleted { target, message_id: id });
    Ok(StatusCode::NO_CONTENT)
}

async fn attachment(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<Response, BanterError> {
    let (filename, bytes) = state.store.read_attachment(account.id, id)?;
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
    Ok(Json(state.store.unread_mentions(account.id)?))
}

async fn read_mention(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<StatusCode, BanterError> {
    state.store.mark_mention_read(account.id, id)?;
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
    state.live.join_server_voice(&state.store, account.id, body.channel_id)?;
    Ok(Json(state.live.voice_channel_members(body.channel_id)))
}

async fn leave_voice(State(state): State<HttpState>, Auth(account): Auth) -> StatusCode {
    state.live.leave_voice(account.id);
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
    Ok(Json(state.live.start_dm_call(&state.store, account.id, id)?))
}

async fn accept_call(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<Json<CallSnapshot>, BanterError> {
    Ok(Json(state.live.accept_dm_call(&state.store, account.id, id)?))
}

async fn decline_call(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<StatusCode, BanterError> {
    state.live.decline_dm_call(account.id, id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn end_call(
    State(state): State<HttpState>,
    Auth(account): Auth,
    Path(id): Path<i64>,
) -> Result<StatusCode, BanterError> {
    state.live.end_dm_call(account.id, id)?;
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
    let account = state.store.session_account(&query.token)?;
    Ok(upgrade.on_upgrade(move |socket| handle_socket(state, account.id, socket)))
}

async fn handle_socket(state: HttpState, user_id: i64, socket: WebSocket) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let socket_id = state.live.register_socket(user_id, tx);
    state.live.connect(user_id, now_ms());
    if let Ok(audience) = state.store.related_user_ids(user_id) {
        state.live.send_event(
            &audience,
            &ServerEvent::Presence {
                user_id,
                state: Presence::Online,
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
                        note_ws_activity(&state, user_id);
                    }
                    let signal_for_room = signal.clone();
                    if state.live.handle_signal(user_id, signal).is_ok() {
                        apply_voice_signal(&state, user_id, signal_for_room);
                    }
                } else if text.contains("activity") {
                    note_ws_activity(&state, user_id);
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
    if let Ok(audience) = state.store.related_user_ids(user_id) {
        state.live.send_event(
            &audience,
            &ServerEvent::Presence {
                user_id,
                state: Presence::Offline,
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

fn note_ws_activity(state: &HttpState, user_id: i64) {
    state.live.note_activity(user_id, now_ms());
    if let Ok(audience) = state.store.related_user_ids(user_id) {
        state.live.send_event(
            &audience,
            &ServerEvent::Presence {
                user_id,
                state: state.live.presence(user_id),
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
