//! Bot intent checks. A missing intent is not an error: the HTTP layer returns JSON null.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::error::BanterError;
use crate::model::{Account, ChatTarget, ServerEvent, Session};
use crate::store::{self, Store};

pub const INTENTS: &[&str] = &[
    "view_channels",
    "read_messages",
    "send_messages",
    "manage_messages",
    "connect_voice",
    "presence",
    "interactions",
    "friends",
    "servers",
    "roles",
    "plugins",
    "debug",
    "calls",
    "board",
    "profile",
];

pub fn known_intent(name: &str) -> bool {
    INTENTS.contains(&name)
}

/// `true` when a bot call must return JSON null and change nothing.
///
/// Humans always pass. A bot passes when the intent is on its checklist and, for a
/// guild route, on the snapshot the admin accepted for that server.
pub fn call_is_null(is_bot: bool, declared: &[String], accepted: Option<&[String]>, intent: &str) -> bool {
    if !is_bot {
        return false;
    }
    if !declared.iter().any(|item| item == intent) {
        return true;
    }
    match accepted {
        Some(list) => !list.iter().any(|item| item == intent),
        None => false,
    }
}

/// What a websocket frame should do for a bot socket.
pub enum EventGate {
    Deliver,
    Drop,
    Need(&'static str),
}

pub fn event_gate(event: &ServerEvent) -> EventGate {
    match event {
        ServerEvent::Ready { .. } => EventGate::Deliver,
        ServerEvent::MessageCreated { .. } | ServerEvent::MessageEdited { .. } | ServerEvent::MessageDeleted { .. } => {
            EventGate::Need("read_messages")
        }
        ServerEvent::Presence { .. } => EventGate::Need("presence"),
        ServerEvent::Voice { .. } => EventGate::Need("connect_voice"),
        ServerEvent::Call { .. } | ServerEvent::CallEnded { .. } => EventGate::Need("calls"),
        ServerEvent::BoardChanged { .. } => EventGate::Need("board"),
        ServerEvent::FriendRequest { .. } | ServerEvent::FriendsChanged => EventGate::Need("friends"),
        ServerEvent::RtcOffer { .. }
        | ServerEvent::RtcAnswer { .. }
        | ServerEvent::RtcIce { .. }
        | ServerEvent::SubscribeLayer { .. }
        | ServerEvent::WantedLayers { .. } => EventGate::Drop,
        ServerEvent::PluginSync { .. } => EventGate::Deliver,
    }
}

/// Sliding windows for bot HTTP limits.
#[derive(Default)]
pub struct Rates {
    hits: HashMap<String, Vec<Instant>>,
}

impl Rates {
    /// `Some(retry_after_ms)` when the key is over `limit` inside `window`.
    pub fn hit(&mut self, key: &str, limit: usize, window: Duration) -> Option<u64> {
        let now = Instant::now();
        let hits = self.hits.entry(key.to_string()).or_default();
        hits.retain(|at| now.duration_since(*at) < window);
        if hits.len() >= limit {
            let oldest = hits.first().copied().unwrap_or(now);
            let wait = window.saturating_sub(now.duration_since(oldest));
            return Some(wait.as_millis() as u64);
        }
        hits.push(now);
        None
    }
}

impl Store {
    pub async fn create_bot(
        &self,
        owner_id: i64,
        username: &str,
        display_name: &str,
        intents: &[String],
    ) -> Result<Session, BanterError> {
        if intents.iter().any(|intent| !known_intent(intent)) {
            return Err(BanterError::Invalid);
        }
        let username = store::normalize_username(username)?;
        let display_name = store::clean_display(display_name)?;
        let now = store::now_ms();
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO users (username, display_name, password_hash, avatar_file, bot, created_at)
             VALUES ($1, $2, NULL, NULL, TRUE, $3) RETURNING id",
        )
        .bind(&username)
        .bind(&display_name)
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .map_err(|err| store::constraint(err, BanterError::UsernameTaken))?;
        let encoded = serde_json::to_string(intents).map_err(|_| BanterError::Invalid)?;
        sqlx::query("INSERT INTO bots (user_id, owner_id, intents, interactions_url) VALUES ($1, $2, $3, NULL)")
            .bind(id)
            .bind(owner_id)
            .bind(&encoded)
            .execute(&self.pool)
            .await
            .map_err(store::db_err)?;
        let token = self.rotate_bot_token(owner_id, id).await?;
        let account = store::account_by_id(&self.pool, id).await?;
        Ok(Session { token, account })
    }

    pub async fn rotate_bot_token(&self, owner_id: i64, bot_id: i64) -> Result<String, BanterError> {
        self.require_bot_owner(owner_id, bot_id).await?;
        sqlx::query("DELETE FROM bot_tokens WHERE user_id = $1")
            .bind(bot_id)
            .execute(&self.pool)
            .await
            .map_err(store::db_err)?;
        let token = store::random_token();
        sqlx::query("INSERT INTO bot_tokens (token, user_id, created_at) VALUES ($1, $2, $3)")
            .bind(&token)
            .bind(bot_id)
            .bind(store::now_ms())
            .execute(&self.pool)
            .await
            .map_err(store::db_err)?;
        Ok(token)
    }

    pub async fn install_bot(&self, user_id: i64, server_id: i64, bot_id: i64) -> Result<(), BanterError> {
        self.require_flag(user_id, server_id, crate::roles::PERM_MANAGE_BOTS).await?;
        let intents = self.bot_intents(bot_id).await?;
        let encoded = serde_json::to_string(&intents).map_err(|_| BanterError::Invalid)?;
        let joined: Option<i64> = sqlx::query_scalar(
            "SELECT user_id FROM memberships WHERE server_id = $1 AND user_id = $2",
        )
        .bind(server_id)
        .bind(bot_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(store::db_err)?;
        if joined.is_none() {
            sqlx::query("INSERT INTO memberships (server_id, user_id, nickname) VALUES ($1, $2, NULL)")
                .bind(server_id)
                .bind(bot_id)
                .execute(&self.pool)
                .await
                .map_err(store::db_err)?;
        }
        sqlx::query(
            "INSERT INTO server_bots (server_id, bot_user_id, accepted_intents) VALUES ($1, $2, $3)
             ON CONFLICT (server_id, bot_user_id) DO UPDATE SET accepted_intents = EXCLUDED.accepted_intents",
        )
        .bind(server_id)
        .bind(bot_id)
        .bind(&encoded)
        .execute(&self.pool)
        .await
        .map_err(store::db_err)?;
        Ok(())
    }

    pub async fn bot_intents(&self, bot_id: i64) -> Result<Vec<String>, BanterError> {
        let raw: Option<String> = sqlx::query_scalar("SELECT intents FROM bots WHERE user_id = $1")
            .bind(bot_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(store::db_err)?;
        let Some(raw) = raw else {
            return Err(BanterError::NotFound);
        };
        serde_json::from_str(&raw).map_err(|_| BanterError::Invalid)
    }

    pub async fn bot_call_is_null(&self, account: &Account, intent: &str, server_id: Option<i64>) -> Result<bool, BanterError> {
        if !account.bot {
            return Ok(false);
        }
        let declared = self.bot_intents(account.id).await?;
        let accepted = match server_id {
            Some(server_id) => {
                let raw: Option<String> = sqlx::query_scalar(
                    "SELECT accepted_intents FROM server_bots WHERE server_id = $1 AND bot_user_id = $2",
                )
                .bind(server_id)
                .bind(account.id)
                .fetch_optional(&self.pool)
                .await
                .map_err(store::db_err)?;
                let Some(raw) = raw else {
                    return Ok(true);
                };
                let list: Vec<String> = serde_json::from_str(&raw).map_err(|_| BanterError::Invalid)?;
                Some(list)
            }
            None => None,
        };
        Ok(call_is_null(true, &declared, accepted.as_deref(), intent))
    }

    pub async fn target_server(&self, target: ChatTarget) -> Result<Option<i64>, BanterError> {
        match target {
            ChatTarget::Channel(channel_id) => {
                let server_id: Option<i64> = sqlx::query_scalar("SELECT server_id FROM channels WHERE id = $1")
                    .bind(channel_id)
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(store::db_err)?;
                Ok(server_id)
            }
            ChatTarget::Direct(_) => Ok(None),
        }
    }

    pub async fn bots_owned(&self, owner_id: i64) -> Result<Vec<(Account, Vec<String>)>, BanterError> {
        let rows = sqlx::query("SELECT user_id, intents FROM bots WHERE owner_id = $1 ORDER BY user_id")
            .bind(owner_id)
            .fetch_all(&self.pool)
            .await
            .map_err(store::db_err)?;
        let mut bots = Vec::new();
        for row in rows {
            use sqlx::Row;
            let user_id: i64 = row.get("user_id");
            let raw: String = row.get("intents");
            let intents: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
            bots.push((store::account_by_id(&self.pool, user_id).await?, intents));
        }
        Ok(bots)
    }

    async fn require_bot_owner(&self, owner_id: i64, bot_id: i64) -> Result<(), BanterError> {
        let found: Option<i64> = sqlx::query_scalar("SELECT owner_id FROM bots WHERE user_id = $1")
            .bind(bot_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(store::db_err)?;
        if found == Some(owner_id) { Ok(()) } else { Err(BanterError::NotFound) }
    }
}
