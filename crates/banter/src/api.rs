//! HTTP client for the local Banter server.

use std::io::Read;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::profiler;
use banter_server::{
    format_banter_notice, Account, BanterVersionReport, CallSnapshot, Category, CategoryLayout,
    Channel, ChannelKind, ChatTarget, CommandAlias, CosmeticItem, DirectChat, Friend,
    FriendRequest, GuildRole, Mention, Message, PluginInfo, Presence, ProfilePatch, ServerDebug,
    ServerCommandSettings, ServerFolder, ServerFolderPatch, ServerHome, ServerRail,
    ServerRailServer, ServerSummary, Session, UserProfile, VoiceFlags, VoiceMember, VoiceRoster,
    ERR_HTTP_PARSE, ERR_HTTP_STATUS, ERR_HTTP_TRANSPORT, ERR_VC_FLAGS, ERR_VC_JOIN, ERR_VC_LEAVE,
};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::Deserialize;

/// Client-side API failure with a catalog code (BA-88). Display is short user text + code.
#[derive(Debug)]
pub struct ApiError {
    code: String,
    message: String,
    detail: Option<String>,
    server_code: Option<String>,
}

impl ApiError {
    fn new(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            code: ERR_HTTP_PARSE.to_string(),
            message: "couldn't complete that request".into(),
            detail: Some(message),
            server_code: None,
        }
    }

    fn from_issue(code: &'static str, message: impl Into<String>, detail: Option<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            detail,
            server_code: None,
        }
    }

    /// Relabel this failure for a specific call site (voice flags, join, …).
    /// Keeps a server-provided message when present; otherwise uses the call-site text.
    fn context(self, code: &'static str, message: impl Into<String>) -> Self {
        let call_site = message.into();
        let (message, detail) = if self.server_code.is_some() {
            let detail = self
                .detail
                .or_else(|| Some(format!("{call_site} [{code}]")));
            (self.message, detail)
        } else {
            let prior = self.detail.unwrap_or(self.message);
            (call_site, Some(prior))
        };
        Self {
            code: code.to_string(),
            message,
            detail,
            server_code: self.server_code,
        }
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    pub fn server_code(&self) -> Option<&str> {
        self.server_code.as_deref()
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&format_banter_notice(&self.code, &self.message))
    }
}

impl std::error::Error for ApiError {}

/// Shared client with a short timeout. Shell list, message, and avatar GETs run
/// on a background worker. Each call still times out so a stuck read cannot
/// occupy that worker forever.
fn http() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout(Duration::from_millis(800))
            .build()
    })
}

/// Longer timeout for register/login. Those endpoints run Argon2 on the server;
/// 800 ms is enough for polls but fails under CI load (`timed out reading response`
/// on `/v1/register`). Matches the foundation suite's 5 s HTTP client.
fn http_auth() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(5))
            .build()
    })
}

#[derive(Clone, Debug, Deserialize)]
pub struct MemberView {
    pub user_id: i64,
    pub username: String,
    pub display_name: String,
    pub nickname: Option<String>,
    pub is_owner: bool,
    #[serde(default)]
    pub has_avatar: bool,
    #[serde(default)]
    pub avatar_rev: i64,
    pub presence: Presence,
    #[serde(default)]
    pub custom_status: Option<String>,
    #[serde(default)]
    pub roles: Vec<GuildRole>,
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
    #[serde(default)]
    code: Option<String>,
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

    pub fn set_base(&mut self, base: impl Into<String>) {
        self.base = base.into().trim_end_matches('/').to_string();
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn register(
        &mut self,
        username: &str,
        display_name: &str,
        password: &str,
    ) -> Result<Session, ApiError> {
        let session: Session = self.send_json(
            http_auth().post(&self.url("/v1/register")),
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
            http_auth().post(&self.url("/v1/login")),
            serde_json::json!({ "username": username, "password": password }),
        )?;
        self.token = Some(session.token.clone());
        Ok(session)
    }

    pub fn logout(&self) -> Result<(), ApiError> {
        self.call_empty(
            "POST",
            "/v1/logout",
            self.auth(http().post(&self.url("/v1/logout"))),
        )
    }

    /// Unauthenticated health/version probe (`GET /v1/health`).
    pub fn health(&self) -> Result<BanterVersionReport, ApiError> {
        self.get_json("/v1/health")
    }

    pub fn server_debug(&self) -> Result<ServerDebug, ApiError> {
        self.get_json("/v1/debug")
    }

    pub fn me(&self) -> Result<Account, ApiError> {
        self.get_json("/v1/me")
    }

    pub fn user_profile(
        &self,
        user_id: i64,
        server_id: Option<i64>,
    ) -> Result<UserProfile, ApiError> {
        let path = match server_id {
            Some(server_id) => format!("/v1/users/{user_id}?server_id={server_id}"),
            None => format!("/v1/users/{user_id}"),
        };
        self.get_json(&path)
    }

    pub fn set_display_name(&self, display_name: &str) -> Result<Account, ApiError> {
        self.send_json(
            self.auth(http().patch(&self.url("/v1/me"))),
            serde_json::json!({ "display_name": display_name }),
        )
    }

    pub fn update_profile(&self, patch: &ProfilePatch) -> Result<UserProfile, ApiError> {
        let body = serde_json::to_value(patch).map_err(|err| ApiError::new(err.to_string()))?;
        self.send_json(self.auth(http().patch(&self.url("/v1/me/profile"))), body)
    }

    pub fn profile_cosmetics(&self) -> Result<Vec<CosmeticItem>, ApiError> {
        self.get_json("/v1/profile/cosmetics")
    }

    pub fn profile_badges(&self) -> Result<Vec<banter_server::BadgeItem>, ApiError> {
        self.get_json("/v1/profile/badges")
    }

    pub fn set_avatar(&self, bytes: &[u8]) -> Result<(), ApiError> {
        let start = Instant::now();
        let result = self
            .auth(http().post(&self.url("/v1/me/avatar")))
            .set("Content-Type", "application/octet-stream")
            .send_bytes(bytes);
        profiler::record_http("POST", "/v1/me/avatar", start.elapsed(), bytes.len(), 0);
        self.finish("POST", "/v1/me/avatar", result)
    }

    pub fn clear_avatar(&self) -> Result<(), ApiError> {
        self.call_empty(
            "DELETE",
            "/v1/me/avatar",
            self.auth(http().delete(&self.url("/v1/me/avatar"))),
        )
    }

    pub fn set_banner(&self, bytes: &[u8]) -> Result<(), ApiError> {
        let start = Instant::now();
        let result = self
            .auth(http().post(&self.url("/v1/me/banner")))
            .set("Content-Type", "application/octet-stream")
            .send_bytes(bytes);
        profiler::record_http("POST", "/v1/me/banner", start.elapsed(), bytes.len(), 0);
        self.finish("POST", "/v1/me/banner", result)
    }

    pub fn clear_banner(&self) -> Result<(), ApiError> {
        self.call_empty(
            "DELETE",
            "/v1/me/banner",
            self.auth(http().delete(&self.url("/v1/me/banner"))),
        )
    }

    pub fn avatar(&self, user_id: i64) -> Result<Vec<u8>, ApiError> {
        let path = format!("/v1/users/{user_id}/avatar");
        let start = Instant::now();
        let response = self
            .auth(http().get(&self.url(&path)))
            .call()
            .map_err(|err| self.http_error("GET", &path, err))?;
        let bytes = response
            .into_reader()
            .bytes()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| ApiError::new(err.to_string()))?;
        profiler::record_http("GET", &path, start.elapsed(), 0, bytes.len());
        Ok(bytes)
    }

    pub fn banner(&self, user_id: i64) -> Result<Vec<u8>, ApiError> {
        let path = format!("/v1/users/{user_id}/banner");
        let start = Instant::now();
        let response = self
            .auth(http().get(&self.url(&path)))
            .call()
            .map_err(|err| self.http_error("GET", &path, err))?;
        let bytes = response
            .into_reader()
            .bytes()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| ApiError::new(err.to_string()))?;
        profiler::record_http("GET", &path, start.elapsed(), 0, bytes.len());
        Ok(bytes)
    }

    pub fn friend_requests(&self) -> Result<(Vec<FriendRequest>, Vec<FriendRequest>), ApiError> {
        let lists: RequestLists = self.get_json("/v1/friends/requests")?;
        Ok((lists.incoming, lists.outgoing))
    }

    pub fn send_friend_request(&self, username: &str) -> Result<FriendRequest, ApiError> {
        self.send_json(
            self.auth(http().post(&self.url("/v1/friends/requests"))),
            serde_json::json!({ "username": username }),
        )
    }

    pub fn accept_friend_request(&self, id: i64) -> Result<(), ApiError> {
        let path = format!("/v1/friends/requests/{id}/accept");
        self.call_empty("POST", &path, self.auth(http().post(&self.url(&path))))
    }

    pub fn decline_friend_request(&self, id: i64) -> Result<(), ApiError> {
        let path = format!("/v1/friends/requests/{id}/decline");
        self.call_empty("POST", &path, self.auth(http().post(&self.url(&path))))
    }

    pub fn friends(&self) -> Result<Vec<Friend>, ApiError> {
        self.get_json("/v1/friends")
    }

    pub fn presence(&self) -> Result<Vec<(i64, Presence, Option<String>)>, ApiError> {
        #[derive(Deserialize)]
        struct Row {
            user_id: i64,
            state: Presence,
            #[serde(default)]
            custom_status: Option<String>,
        }
        let rows: Vec<Row> = self.get_json("/v1/presence")?;
        Ok(rows
            .into_iter()
            .map(|row| (row.user_id, row.state, row.custom_status))
            .collect())
    }

    pub fn servers(&self) -> Result<Vec<ServerSummary>, ApiError> {
        self.get_json("/v1/servers")
    }

    pub fn server_rail(&self) -> Result<ServerRail, ApiError> {
        self.get_json("/v1/me/server-rail")
    }

    pub fn create_server_folder(
        &self,
        name: &str,
        parent_id: Option<i64>,
        color: i64,
    ) -> Result<ServerFolder, ApiError> {
        self.send_json(
            self.auth(http().post(&self.url("/v1/me/server-folders"))),
            serde_json::json!({
                "name": name,
                "parent_id": parent_id,
                "color": color,
            }),
        )
    }

    pub fn update_server_folder(
        &self,
        folder_id: i64,
        patch: ServerFolderPatch,
    ) -> Result<ServerFolder, ApiError> {
        let body = serde_json::to_value(patch).map_err(|err| ApiError::new(err.to_string()))?;
        self.send_json(
            self.auth(http().patch(&self.url(&format!("/v1/me/server-folders/{folder_id}")))),
            body,
        )
    }

    pub fn delete_server_folder(&self, folder_id: i64) -> Result<(), ApiError> {
        let path = format!("/v1/me/server-folders/{folder_id}");
        self.call_empty("DELETE", &path, self.auth(http().delete(&self.url(&path))))
    }

    pub fn move_server_to_folder(
        &self,
        server_id: i64,
        folder_id: Option<i64>,
        position: i64,
    ) -> Result<ServerRailServer, ApiError> {
        self.send_json(
            self.auth(http().put(&self.url(&format!("/v1/me/servers/{server_id}/folder")))),
            serde_json::json!({
                "folder_id": folder_id,
                "position": position,
            }),
        )
    }

    pub fn list_plugins(&self, server_id: i64) -> Result<Vec<PluginInfo>, ApiError> {
        self.get_json(&format!("/v1/servers/{server_id}/plugins"))
    }

    pub fn remove_plugin(&self, server_id: i64, plugin_id: &str) -> Result<(), ApiError> {
        let path = format!("/v1/servers/{server_id}/plugins/{plugin_id}");
        self.call_empty("DELETE", &path, self.auth(http().delete(&self.url(&path))))
    }

    pub fn install_plugin(&self, server_id: i64, zip_bytes: &[u8]) -> Result<PluginInfo, ApiError> {
        let path = format!("/v1/servers/{server_id}/plugins");
        let start = Instant::now();
        let request = self
            .auth(http().post(&self.url(&path)))
            .set("Content-Type", "application/zip");
        let response = request
            .send_bytes(zip_bytes)
            .map_err(|err| self.http_error("POST", &path, err))?;
        let value = response.into_json().map_err(|err| {
            ApiError::from_issue(
                ERR_HTTP_PARSE,
                "couldn't read the server reply",
                Some(err.to_string()),
            )
        })?;
        profiler::record_http("POST", &path, start.elapsed(), zip_bytes.len(), 0);
        Ok(value)
    }

    pub fn create_server(&self, name: &str) -> Result<ServerHome, ApiError> {
        self.send_json(
            self.auth(http().post(&self.url("/v1/servers"))),
            serde_json::json!({ "name": name }),
        )
    }

    pub fn create_invite(&self, server_id: i64) -> Result<String, ApiError> {
        let body: InviteBody = self.send_json(
            self.auth(http().post(&self.url(&format!("/v1/servers/{server_id}/invites")))),
            serde_json::json!({}),
        )?;
        Ok(body.code)
    }

    pub fn join_invite(&self, code: &str) -> Result<ServerHome, ApiError> {
        self.send_json(
            self.auth(http().post(&self.url(&format!("/v1/invites/{code}/join")))),
            serde_json::json!({}),
        )
    }

    pub fn members(&self, server_id: i64) -> Result<Vec<MemberView>, ApiError> {
        self.get_json(&format!("/v1/servers/{server_id}/members"))
    }

    pub fn roles(&self, server_id: i64) -> Result<Vec<GuildRole>, ApiError> {
        self.get_json(&format!("/v1/servers/{server_id}/roles"))
    }

    pub fn create_role(
        &self,
        server_id: i64,
        name: &str,
        color: i64,
        hoist: bool,
        permissions: i64,
    ) -> Result<GuildRole, ApiError> {
        self.send_json(
            self.auth(http().post(&self.url(&format!("/v1/servers/{server_id}/roles")))),
            serde_json::json!({
                "name": name,
                "color": color,
                "hoist": hoist,
                "permissions": permissions,
            }),
        )
    }

    pub fn update_role(
        &self,
        server_id: i64,
        role_id: i64,
        name: &str,
        color: i64,
        hoist: bool,
        permissions: i64,
    ) -> Result<GuildRole, ApiError> {
        self.send_json(
            self.auth(http().patch(&self.url(&format!("/v1/servers/{server_id}/roles/{role_id}")))),
            serde_json::json!({
                "name": name,
                "color": color,
                "hoist": hoist,
                "permissions": permissions,
            }),
        )
    }

    pub fn delete_role(&self, server_id: i64, role_id: i64) -> Result<(), ApiError> {
        let path = format!("/v1/servers/{server_id}/roles/{role_id}");
        self.call_empty("DELETE", &path, self.auth(http().delete(&self.url(&path))))
    }

    pub fn set_member_roles(
        &self,
        server_id: i64,
        user_id: i64,
        role_ids: &[i64],
    ) -> Result<(), ApiError> {
        let path = format!("/v1/servers/{server_id}/members/{user_id}/roles");
        self.send_json_empty(
            "PUT",
            &path,
            self.auth(http().put(&self.url(&path))),
            serde_json::json!({ "role_ids": role_ids }),
        )
    }

    pub fn set_nickname(&self, server_id: i64, nickname: Option<&str>) -> Result<(), ApiError> {
        let path = format!("/v1/servers/{server_id}/nickname");
        self.send_json_empty(
            "PUT",
            &path,
            self.auth(http().put(&self.url(&path))),
            serde_json::json!({ "nickname": nickname }),
        )
    }

    pub fn create_category(&self, server_id: i64, name: &str) -> Result<Category, ApiError> {
        self.send_json(
            self.auth(http().post(&self.url(&format!("/v1/servers/{server_id}/categories")))),
            serde_json::json!({ "name": name }),
        )
    }

    pub fn create_channel(
        &self,
        server_id: i64,
        category_id: i64,
        name: &str,
        kind: ChannelKind,
        icon: Option<&str>,
    ) -> Result<Channel, ApiError> {
        self.send_json(
            self.auth(http().post(&self.url(&format!("/v1/servers/{server_id}/channels")))),
            serde_json::json!({
                "category_id": category_id,
                "name": name,
                "kind": kind,
                "icon": icon,
            }),
        )
    }

    pub fn update_channel(
        &self,
        server_id: i64,
        channel_id: i64,
        name: &str,
        icon: Option<&str>,
    ) -> Result<Channel, ApiError> {
        self.send_json(
            self.auth(
                http().patch(&self.url(&format!("/v1/servers/{server_id}/channels/{channel_id}"))),
            ),
            serde_json::json!({ "name": name, "icon": icon }),
        )
    }

    pub fn layout(&self, server_id: i64) -> Result<Vec<CategoryLayout>, ApiError> {
        self.get_json(&format!("/v1/servers/{server_id}/layout"))
    }

    pub fn command_settings(&self, server_id: i64) -> Result<ServerCommandSettings, ApiError> {
        self.get_json(&format!("/v1/servers/{server_id}/commands"))
    }

    pub fn set_command_settings(
        &self,
        server_id: i64,
        prefix: &str,
        aliases: &[CommandAlias],
    ) -> Result<ServerCommandSettings, ApiError> {
        self.send_json(
            self.auth(http().put(&self.url(&format!("/v1/servers/{server_id}/commands")))),
            serde_json::json!({ "prefix": prefix, "aliases": aliases }),
        )
    }

    pub fn server_voice(&self, server_id: i64) -> Result<Vec<VoiceRoster>, ApiError> {
        self.get_json(&format!("/v1/servers/{server_id}/voice"))
    }

    pub fn messages(
        &self,
        target: ChatTarget,
        limit: u32,
        query: Option<&str>,
    ) -> Result<Vec<Message>, ApiError> {
        let path = target_messages_path(target);
        let start = Instant::now();
        let mut request = self
            .auth(http().get(&self.url(&path)))
            .query("limit", &limit.to_string());
        if let Some(query) = query.map(str::trim).filter(|query| !query.is_empty()) {
            request = request.query("q", query);
        }
        let response = request
            .call()
            .map_err(|err| self.http_error("GET", &path, err))?;
        let resp_hint = content_length(&response);
        let value = response
            .into_json()
            .map_err(|err| ApiError::new(err.to_string()))?;
        profiler::record_http("GET", &path, start.elapsed(), 0, resp_hint);
        Ok(value)
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
            self.auth(http().post(&self.url("/v1/messages"))),
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
            self.auth(http().patch(&self.url(&format!("/v1/messages/{id}")))),
            serde_json::json!({ "body": body }),
        )
    }

    pub fn delete_message(&self, id: i64) -> Result<(), ApiError> {
        let path = format!("/v1/messages/{id}");
        self.call_empty("DELETE", &path, self.auth(http().delete(&self.url(&path))))
    }

    pub fn attachment(&self, id: i64) -> Result<Vec<u8>, ApiError> {
        let path = format!("/v1/attachments/{id}");
        let start = Instant::now();
        let response = self
            .auth(http().get(&self.url(&path)))
            .call()
            .map_err(|err| self.http_error("GET", &path, err))?;
        let bytes = response
            .into_reader()
            .bytes()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| ApiError::new(err.to_string()))?;
        profiler::record_http("GET", &path, start.elapsed(), 0, bytes.len());
        Ok(bytes)
    }

    pub fn mentions(&self) -> Result<Vec<Mention>, ApiError> {
        self.get_json("/v1/mentions")
    }

    pub fn mark_mention_read(&self, id: i64) -> Result<(), ApiError> {
        let path = format!("/v1/mentions/{id}/read");
        self.call_empty("POST", &path, self.auth(http().post(&self.url(&path))))
    }

    pub fn open_dm(&self, user_id: i64) -> Result<DirectChat, ApiError> {
        self.send_json(
            self.auth(http().post(&self.url("/v1/dms"))),
            serde_json::json!({ "user_id": user_id }),
        )
    }

    pub fn direct_messages(&self) -> Result<Vec<DirectChat>, ApiError> {
        self.get_json("/v1/dms")
    }

    pub fn join_voice(&self, channel_id: i64) -> Result<Vec<VoiceMember>, ApiError> {
        self.send_json_at(
            "POST",
            "/v1/voice/join",
            self.auth(http().post(&self.url("/v1/voice/join"))),
            serde_json::json!({ "channel_id": channel_id }),
        )
        .map_err(|err| err.context(ERR_VC_JOIN, "couldn't join voice"))
    }

    pub fn leave_voice(&self) -> Result<(), ApiError> {
        self.call_empty(
            "POST",
            "/v1/voice/leave",
            self.auth(http().post(&self.url("/v1/voice/leave"))),
        )
        .map_err(|err| err.context(ERR_VC_LEAVE, "couldn't leave voice"))
    }

    pub fn set_flags(&self, flags: VoiceFlags) -> Result<(), ApiError> {
        self.send_json_empty(
            "POST",
            "/v1/voice/flags",
            self.auth(http().post(&self.url("/v1/voice/flags"))),
            serde_json::json!(flags),
        )
        .map_err(|err| err.context(ERR_VC_FLAGS, "couldn't update voice state"))
    }

    pub fn start_call(&self, dm_id: i64) -> Result<CallSnapshot, ApiError> {
        self.send_json(
            self.auth(http().post(&self.url(&format!("/v1/dms/{dm_id}/call")))),
            serde_json::json!({}),
        )
    }

    pub fn accept_call(&self, dm_id: i64) -> Result<CallSnapshot, ApiError> {
        self.send_json(
            self.auth(http().post(&self.url(&format!("/v1/dms/{dm_id}/call/accept")))),
            serde_json::json!({}),
        )
    }

    pub fn decline_call(&self, dm_id: i64) -> Result<(), ApiError> {
        let path = format!("/v1/dms/{dm_id}/call/decline");
        self.call_empty("POST", &path, self.auth(http().post(&self.url(&path))))
    }

    pub fn end_call(&self, dm_id: i64) -> Result<(), ApiError> {
        let path = format!("/v1/dms/{dm_id}/call/end");
        self.call_empty("POST", &path, self.auth(http().post(&self.url(&path))))
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
        let start = Instant::now();
        let response = self
            .auth(http().get(&self.url(path)))
            .call()
            .map_err(|err| self.http_error("GET", path, err))?;
        let resp_hint = content_length(&response);
        let value = response.into_json().map_err(|err| {
            ApiError::from_issue(
                ERR_HTTP_PARSE,
                "couldn't read the server reply",
                Some(err.to_string()),
            )
        })?;
        profiler::record_http("GET", path, start.elapsed(), 0, resp_hint);
        Ok(value)
    }

    fn send_json<T: serde::de::DeserializeOwned>(
        &self,
        request: ureq::Request,
        body: serde_json::Value,
    ) -> Result<T, ApiError> {
        let method = request.method().to_string();
        let path = path_from_url(request.url());
        self.send_json_at(&method, &path, request, body)
    }

    fn send_json_at<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        request: ureq::Request,
        body: serde_json::Value,
    ) -> Result<T, ApiError> {
        let start = Instant::now();
        let req_bytes = serde_json::to_vec(&body)
            .map(|bytes| bytes.len())
            .unwrap_or(0);
        let response = request
            .send_json(body)
            .map_err(|err| self.http_error(method, path, err))?;
        let resp_hint = content_length(&response);
        let value = response.into_json().map_err(|err| {
            ApiError::from_issue(
                ERR_HTTP_PARSE,
                "couldn't read the server reply",
                Some(err.to_string()),
            )
        })?;
        profiler::record_http(method, path, start.elapsed(), req_bytes, resp_hint);
        Ok(value)
    }

    fn send_json_empty(
        &self,
        method: &str,
        path: &str,
        request: ureq::Request,
        body: serde_json::Value,
    ) -> Result<(), ApiError> {
        let start = Instant::now();
        let req_bytes = serde_json::to_vec(&body)
            .map(|bytes| bytes.len())
            .unwrap_or(0);
        let result = request.send_json(body);
        profiler::record_http(method, path, start.elapsed(), req_bytes, 0);
        self.finish(method, path, result)
    }

    fn call_empty(&self, method: &str, path: &str, request: ureq::Request) -> Result<(), ApiError> {
        let start = Instant::now();
        let result = request.call();
        profiler::record_http(method, path, start.elapsed(), 0, 0);
        self.finish(method, path, result)
    }

    fn finish(
        &self,
        method: &str,
        path: &str,
        result: Result<ureq::Response, ureq::Error>,
    ) -> Result<(), ApiError> {
        result
            .map(|_| ())
            .map_err(|err| self.http_error(method, path, err))
    }

    fn http_error(&self, method: &str, path: &str, error: ureq::Error) -> ApiError {
        match error {
            ureq::Error::Status(status, response) => {
                let body = response.into_string().unwrap_or_default();
                if let Ok(parsed) = serde_json::from_str::<ErrorBody>(&body) {
                    ApiError {
                        code: parsed
                            .code
                            .clone()
                            .unwrap_or_else(|| ERR_HTTP_STATUS.to_string()),
                        message: parsed.error,
                        detail: Some(format!("{method} {path} → HTTP {status}")),
                        server_code: parsed.code,
                    }
                } else if body.is_empty() {
                    ApiError::from_issue(
                        ERR_HTTP_STATUS,
                        format!("server returned HTTP {status}"),
                        Some(format!("{method} {path} → HTTP {status} empty body")),
                    )
                } else {
                    ApiError::from_issue(
                        ERR_HTTP_STATUS,
                        format!("server returned HTTP {status}"),
                        Some(format!("{method} {path} → HTTP {status}: {body}")),
                    )
                }
            }
            other => ApiError::from_issue(
                ERR_HTTP_TRANSPORT,
                "couldn't reach the Banter server",
                Some(other.to_string()),
            ),
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

fn content_length(response: &ureq::Response) -> usize {
    response
        .header("Content-Length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

fn path_from_url(url: &str) -> String {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);
    let path = rest.find('/').map(|index| &rest[index..]).unwrap_or("/");
    path.split('?').next().unwrap_or(path).to_string()
}
