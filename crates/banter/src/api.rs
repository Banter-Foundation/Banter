//! HTTP client for the local Banter server.

use std::io::Read;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use banter_server::{
    Account, CallSnapshot, Category, CategoryLayout, Channel, ChannelKind, ChatTarget, DirectChat,
    Friend, FriendRequest, Mention, Message, Presence, ServerHome, ServerSummary, Session, VoiceFlags,
    VoiceMember,
};
use serde::Deserialize;

#[derive(Debug)]
pub struct ApiError {
    message: String,
}

impl ApiError {
    fn new(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

#[derive(Clone, Debug, Deserialize)]
pub struct MemberView {
    pub user_id: i64,
    pub username: String,
    pub display_name: String,
    pub nickname: Option<String>,
    pub is_owner: bool,
    pub presence: Presence,
}

#[derive(Deserialize)]
struct RequestLists {
    incoming: Vec<FriendRequest>,
    outgoing: Vec<FriendRequest>,
}

#[derive(Deserialize)]
struct InviteBody {
    code: String,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: String,
}

pub struct Client {
    base: String,
    token: Option<String>,
}

impl Client {
    pub fn new(base: impl Into<String>) -> Self {
        let base = base.into().trim_end_matches('/').to_string();
        Self { base, token: None }
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    pub fn set_token(&mut self, token: Option<String>) {
        self.token = token.filter(|token| !token.is_empty());
    }

    pub fn register(
        &mut self,
        username: &str,
        display_name: &str,
        password: &str,
    ) -> Result<Session, ApiError> {
        let session: Session = self.send_json(
            ureq::post(&self.url("/v1/register")),
            serde_json::json!({
                "username": username,
                "display_name": display_name,
                "password": password,
            }),
        )?;
        self.token = Some(session.token.clone());
        Ok(session)
    }

    pub fn login(&mut self, username: &str, password: &str) -> Result<Session, ApiError> {
        let session: Session = self.send_json(
            ureq::post(&self.url("/v1/login")),
            serde_json::json!({ "username": username, "password": password }),
        )?;
        self.token = Some(session.token.clone());
        Ok(session)
    }

    pub fn logout(&self) -> Result<(), ApiError> {
        self.finish(self.auth(ureq::post(&self.url("/v1/logout"))).call())
    }

    pub fn me(&self) -> Result<Account, ApiError> {
        self.get_json("/v1/me")
    }

    pub fn set_display_name(&self, display_name: &str) -> Result<Account, ApiError> {
        self.send_json(
            self.auth(ureq::patch(&self.url("/v1/me"))),
            serde_json::json!({ "display_name": display_name }),
        )
    }

    pub fn set_avatar(&self, bytes: &[u8]) -> Result<(), ApiError> {
        self.finish(
            self.auth(ureq::post(&self.url("/v1/me/avatar")))
                .set("Content-Type", "application/octet-stream")
                .send_bytes(bytes),
        )
    }

    pub fn avatar(&self, user_id: i64) -> Result<Vec<u8>, ApiError> {
        let response = self
            .auth(ureq::get(&self.url(&format!("/v1/users/{user_id}/avatar"))))
            .call()
            .map_err(|err| self.http_error(err))?;
        response.into_reader().bytes().collect::<Result<Vec<_>, _>>().map_err(|err| ApiError::new(err.to_string()))
    }

    pub fn friend_requests(&self) -> Result<(Vec<FriendRequest>, Vec<FriendRequest>), ApiError> {
        let lists: RequestLists = self.get_json("/v1/friends/requests")?;
        Ok((lists.incoming, lists.outgoing))
    }

    pub fn send_friend_request(&self, username: &str) -> Result<FriendRequest, ApiError> {
        self.send_json(
            self.auth(ureq::post(&self.url("/v1/friends/requests"))),
            serde_json::json!({ "username": username }),
        )
    }

    pub fn accept_friend_request(&self, id: i64) -> Result<(), ApiError> {
        self.finish(self.auth(ureq::post(&self.url(&format!("/v1/friends/requests/{id}/accept")))).call())
    }

    pub fn decline_friend_request(&self, id: i64) -> Result<(), ApiError> {
        self.finish(self.auth(ureq::post(&self.url(&format!("/v1/friends/requests/{id}/decline")))).call())
    }

    pub fn friends(&self) -> Result<Vec<Friend>, ApiError> {
        self.get_json("/v1/friends")
    }

    pub fn presence(&self) -> Result<Vec<(i64, Presence)>, ApiError> {
        #[derive(Deserialize)]
        struct Row {
            user_id: i64,
            state: Presence,
        }
        let rows: Vec<Row> = self.get_json("/v1/presence")?;
        Ok(rows.into_iter().map(|row| (row.user_id, row.state)).collect())
    }

    pub fn servers(&self) -> Result<Vec<ServerSummary>, ApiError> {
        self.get_json("/v1/servers")
    }

    pub fn create_server(&self, name: &str) -> Result<ServerHome, ApiError> {
        self.send_json(
            self.auth(ureq::post(&self.url("/v1/servers"))),
            serde_json::json!({ "name": name }),
        )
    }

    pub fn create_invite(&self, server_id: i64) -> Result<String, ApiError> {
        let body: InviteBody = self.send_json(
            self.auth(ureq::post(&self.url(&format!("/v1/servers/{server_id}/invites")))),
            serde_json::json!({}),
        )?;
        Ok(body.code)
    }

    pub fn join_invite(&self, code: &str) -> Result<ServerHome, ApiError> {
        self.send_json(
            self.auth(ureq::post(&self.url(&format!("/v1/invites/{code}/join")))),
            serde_json::json!({}),
        )
    }

    pub fn members(&self, server_id: i64) -> Result<Vec<MemberView>, ApiError> {
        self.get_json(&format!("/v1/servers/{server_id}/members"))
    }

    pub fn set_nickname(&self, server_id: i64, nickname: Option<&str>) -> Result<(), ApiError> {
        self.send_json_empty(
            self.auth(ureq::put(&self.url(&format!("/v1/servers/{server_id}/nickname")))),
            serde_json::json!({ "nickname": nickname }),
        )
    }

    pub fn create_category(&self, server_id: i64, name: &str) -> Result<Category, ApiError> {
        self.send_json(
            self.auth(ureq::post(&self.url(&format!("/v1/servers/{server_id}/categories")))),
            serde_json::json!({ "name": name }),
        )
    }

    pub fn create_channel(
        &self,
        server_id: i64,
        category_id: i64,
        name: &str,
        kind: ChannelKind,
    ) -> Result<Channel, ApiError> {
        self.send_json(
            self.auth(ureq::post(&self.url(&format!("/v1/servers/{server_id}/channels")))),
            serde_json::json!({ "category_id": category_id, "name": name, "kind": kind }),
        )
    }

    pub fn layout(&self, server_id: i64) -> Result<Vec<CategoryLayout>, ApiError> {
        self.get_json(&format!("/v1/servers/{server_id}/layout"))
    }

    pub fn messages(
        &self,
        target: ChatTarget,
        limit: u32,
        query: Option<&str>,
    ) -> Result<Vec<Message>, ApiError> {
        let mut request = self
            .auth(ureq::get(&self.url(&target_messages_path(target))))
            .query("limit", &limit.to_string());
        if let Some(query) = query.map(str::trim).filter(|query| !query.is_empty()) {
            request = request.query("q", query);
        }
        let response = request.call().map_err(|err| self.http_error(err))?;
        response.into_json().map_err(|err| ApiError::new(err.to_string()))
    }

    pub fn send_message(
        &self,
        target: ChatTarget,
        body: &str,
        reply_to: Option<i64>,
        files: &[(&str, &[u8])],
    ) -> Result<Message, ApiError> {
        let files = files
            .iter()
            .map(|(filename, bytes)| {
                serde_json::json!({
                    "filename": filename,
                    "data_base64": STANDARD.encode(bytes),
                })
            })
            .collect::<Vec<_>>();
        self.send_json(
            self.auth(ureq::post(&self.url("/v1/messages"))),
            serde_json::json!({
                "target": target,
                "body": body,
                "reply_to": reply_to,
                "files": files,
            }),
        )
    }

    pub fn edit_message(&self, id: i64, body: &str) -> Result<Message, ApiError> {
        self.send_json(
            self.auth(ureq::patch(&self.url(&format!("/v1/messages/{id}")))),
            serde_json::json!({ "body": body }),
        )
    }

    pub fn delete_message(&self, id: i64) -> Result<(), ApiError> {
        self.finish(self.auth(ureq::delete(&self.url(&format!("/v1/messages/{id}")))).call())
    }

    pub fn attachment(&self, id: i64) -> Result<Vec<u8>, ApiError> {
        let response = self
            .auth(ureq::get(&self.url(&format!("/v1/attachments/{id}"))))
            .call()
            .map_err(|err| self.http_error(err))?;
        response.into_reader().bytes().collect::<Result<Vec<_>, _>>().map_err(|err| ApiError::new(err.to_string()))
    }

    pub fn mentions(&self) -> Result<Vec<Mention>, ApiError> {
        self.get_json("/v1/mentions")
    }

    pub fn mark_mention_read(&self, id: i64) -> Result<(), ApiError> {
        self.finish(self.auth(ureq::post(&self.url(&format!("/v1/mentions/{id}/read")))).call())
    }

    pub fn open_dm(&self, user_id: i64) -> Result<DirectChat, ApiError> {
        self.send_json(
            self.auth(ureq::post(&self.url("/v1/dms"))),
            serde_json::json!({ "user_id": user_id }),
        )
    }

    pub fn direct_messages(&self) -> Result<Vec<DirectChat>, ApiError> {
        self.get_json("/v1/dms")
    }

    pub fn join_voice(&self, channel_id: i64) -> Result<Vec<VoiceMember>, ApiError> {
        self.send_json(
            self.auth(ureq::post(&self.url("/v1/voice/join"))),
            serde_json::json!({ "channel_id": channel_id }),
        )
    }

    pub fn leave_voice(&self) -> Result<(), ApiError> {
        self.finish(self.auth(ureq::post(&self.url("/v1/voice/leave"))).call())
    }

    pub fn set_flags(&self, flags: VoiceFlags) -> Result<(), ApiError> {
        self.send_json_empty(self.auth(ureq::post(&self.url("/v1/voice/flags"))), serde_json::json!(flags))
    }

    pub fn start_call(&self, dm_id: i64) -> Result<CallSnapshot, ApiError> {
        self.send_json(self.auth(ureq::post(&self.url(&format!("/v1/dms/{dm_id}/call")))), serde_json::json!({}))
    }

    pub fn accept_call(&self, dm_id: i64) -> Result<CallSnapshot, ApiError> {
        self.send_json(
            self.auth(ureq::post(&self.url(&format!("/v1/dms/{dm_id}/call/accept")))),
            serde_json::json!({}),
        )
    }

    pub fn decline_call(&self, dm_id: i64) -> Result<(), ApiError> {
        self.finish(self.auth(ureq::post(&self.url(&format!("/v1/dms/{dm_id}/call/decline")))).call())
    }

    pub fn end_call(&self, dm_id: i64) -> Result<(), ApiError> {
        self.finish(self.auth(ureq::post(&self.url(&format!("/v1/dms/{dm_id}/call/end")))).call())
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn auth(&self, request: ureq::Request) -> ureq::Request {
        if let Some(token) = &self.token {
            request.set("Authorization", &format!("Bearer {token}"))
        } else {
            request
        }
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let response = self.auth(ureq::get(&self.url(path))).call().map_err(|err| self.http_error(err))?;
        response.into_json().map_err(|err| ApiError::new(err.to_string()))
    }

    fn send_json<T: serde::de::DeserializeOwned>(
        &self,
        request: ureq::Request,
        body: serde_json::Value,
    ) -> Result<T, ApiError> {
        let response = request.send_json(body).map_err(|err| self.http_error(err))?;
        response.into_json().map_err(|err| ApiError::new(err.to_string()))
    }

    fn send_json_empty(&self, request: ureq::Request, body: serde_json::Value) -> Result<(), ApiError> {
        self.finish(request.send_json(body))
    }

    fn finish(&self, result: Result<ureq::Response, ureq::Error>) -> Result<(), ApiError> {
        result.map(|_| ()).map_err(|err| self.http_error(err))
    }

    fn http_error(&self, error: ureq::Error) -> ApiError {
        match error {
            ureq::Error::Status(_, response) => {
                let body = response.into_string().unwrap_or_default();
                if let Ok(parsed) = serde_json::from_str::<ErrorBody>(&body) {
                    ApiError::new(parsed.error)
                } else if body.is_empty() {
                    ApiError::new("request failed")
                } else {
                    ApiError::new(body)
                }
            }
            other => ApiError::new(other.to_string()),
        }
    }
}

pub fn websocket_url(http_base: &str, token: &str) -> String {
    let base = http_base.trim_end_matches('/');
    let ws = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        format!("ws://{base}")
    };
    format!("{ws}/v1/ws?token={token}")
}

fn target_messages_path(target: ChatTarget) -> String {
    match target {
        ChatTarget::Channel(id) => format!("/v1/channels/{id}/messages"),
        ChatTarget::Direct(id) => format!("/v1/dms/{id}/messages"),
    }
}
