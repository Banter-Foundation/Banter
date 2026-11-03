use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use rand::Rng;
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::BanterError;
use crate::model::*;

pub struct Store {
    conn: Mutex<Connection>,
    root: PathBuf,
}

impl Store {
    pub fn open(root: &Path) -> Result<Self, BanterError> {
        fs::create_dir_all(root).map_err(|err| BanterError::Storage(err.to_string()))?;
        fs::create_dir_all(root.join("files")).map_err(|err| BanterError::Storage(err.to_string()))?;
        fs::create_dir_all(root.join("avatars")).map_err(|err| BanterError::Storage(err.to_string()))?;
        let conn = Connection::open(root.join("banter.db")).map_err(db_err)?;
        conn.pragma_update(None, "foreign_keys", "ON").map_err(db_err)?;
        conn.pragma_update(None, "journal_mode", "WAL").map_err(db_err)?;
        conn.execute_batch(include_str!("schema.sql")).map_err(db_err)?;
        if let Err(err) = conn.execute("ALTER TABLE users ADD COLUMN avatar_rev INTEGER NOT NULL DEFAULT 0", []) {
            let text = err.to_string().to_lowercase();
            if !text.contains("duplicate column") {
                return Err(db_err(err));
            }
        }
        Ok(Self {
            conn: Mutex::new(conn),
            root: root.to_path_buf(),
        })
    }

    pub fn register(&self, username: &str, display_name: &str, password: &str) -> Result<Account, BanterError> {
        let username = normalize_username(username)?;
        let display_name = clean_display(display_name)?;
        check_password(password)?;
        let hash = hash_password(password)?;
        let now = now_ms();
        self.with_mut(|conn| {
            let taken: Option<i64> = conn
                .query_row("SELECT id FROM users WHERE username = ?1", [&username], |row| row.get(0))
                .optional()
                .map_err(db_err)?;
            if taken.is_some() {
                return Err(BanterError::UsernameTaken);
            }
            conn.execute(
                "INSERT INTO users (username, display_name, password_hash, avatar_file, created_at) VALUES (?1, ?2, ?3, NULL, ?4)",
                params![username, display_name, hash, now],
            )
            .map_err(|err| constraint(err, BanterError::UsernameTaken))?;
            let id = conn.last_insert_rowid();
            account_by_id(conn, id)
        })
    }

    pub fn login(&self, username: &str, password: &str) -> Result<Session, BanterError> {
        let username = normalize_username(username)?;
        let row = self.with(|conn| {
            conn.query_row(
                "SELECT id, password_hash FROM users WHERE username = ?1",
                [&username],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(db_err)
        })?;
        let Some((id, hash)) = row else {
            return Err(BanterError::InvalidCredentials);
        };
        verify_password(password, &hash)?;
        let token = random_token();
        self.with_mut(|conn| {
            conn.execute(
                "INSERT INTO sessions (token, user_id, created_at) VALUES (?1, ?2, ?3)",
                params![token, id, now_ms()],
            )
            .map_err(db_err)?;
            Ok(())
        })?;
        Ok(Session {
            token,
            account: self.account(id)?,
        })
    }

    pub fn logout(&self, token: &str) -> Result<(), BanterError> {
        self.with_mut(|conn| {
            conn.execute("DELETE FROM sessions WHERE token = ?1", [token]).map_err(db_err)?;
            Ok(())
        })
    }

    pub fn session_account(&self, token: &str) -> Result<Account, BanterError> {
        self.with(|conn| {
            let id: Option<i64> = conn
                .query_row("SELECT user_id FROM sessions WHERE token = ?1", [token], |row| row.get(0))
                .optional()
                .map_err(db_err)?;
            let Some(id) = id else {
                return Err(BanterError::InvalidCredentials);
            };
            account_by_id(conn, id)
        })
    }

    pub fn account(&self, user_id: i64) -> Result<Account, BanterError> {
        self.with(|conn| account_by_id(conn, user_id))
    }

    pub fn set_display_name(&self, user_id: i64, display_name: &str) -> Result<Account, BanterError> {
        let display_name = clean_display(display_name)?;
        self.with_mut(|conn| {
            let changed = conn
                .execute("UPDATE users SET display_name = ?1 WHERE id = ?2", params![display_name, user_id])
                .map_err(db_err)?;
            if changed == 0 {
                return Err(BanterError::NotFound);
            }
            account_by_id(conn, user_id)
        })
    }

    pub fn set_avatar(&self, user_id: i64, bytes: &[u8]) -> Result<(), BanterError> {
        if bytes.is_empty() || bytes.len() > MAX_AVATAR_BYTES {
            return Err(BanterError::InvalidAvatar);
        }
        let decoded = image::load_from_memory(bytes).map_err(|_| BanterError::InvalidAvatar)?;
        let scaled = if decoded.width() > 512 || decoded.height() > 512 {
            decoded.thumbnail(512, 512)
        } else {
            decoded
        };
        let mut png = Vec::new();
        scaled
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|err| BanterError::Storage(err.to_string()))?;
        self.account(user_id)?;
        let path = self.root.join("avatars").join(format!("{user_id}.png"));
        fs::write(&path, &png).map_err(|err| BanterError::Storage(err.to_string()))?;
        self.with_mut(|conn| {
            conn.execute(
                "UPDATE users SET avatar_file = ?1, avatar_rev = avatar_rev + 1 WHERE id = ?2",
                params![format!("{user_id}.png"), user_id],
            )
            .map_err(db_err)?;
            Ok(())
        })
    }

    pub fn clear_avatar(&self, user_id: i64) -> Result<(), BanterError> {
        self.account(user_id)?;
        let path = self.root.join("avatars").join(format!("{user_id}.png"));
        if path.exists() {
            fs::remove_file(&path).map_err(|err| BanterError::Storage(err.to_string()))?;
        }
        self.with_mut(|conn| {
            conn.execute(
                "UPDATE users SET avatar_file = NULL, avatar_rev = 0 WHERE id = ?1",
                [user_id],
            )
            .map_err(db_err)?;
            Ok(())
        })
    }

    pub fn avatar_bytes(&self, user_id: i64) -> Result<Vec<u8>, BanterError> {
        let name = self.with(|conn| {
            let file: Option<String> = conn
                .query_row("SELECT avatar_file FROM users WHERE id = ?1", [user_id], |row| row.get(0))
                .optional()
                .map_err(db_err)?
                .ok_or(BanterError::NotFound)?;
            file.ok_or(BanterError::NotFound)
        })?;
        let path = self.root.join("avatars").join(name);
        fs::read(&path).map_err(|_| BanterError::NotFound)
    }

    pub fn send_friend_request(&self, from_user: i64, to_username: &str) -> Result<FriendRequest, BanterError> {
        let to_username = normalize_username(to_username)?;
        self.with_mut(|conn| {
            let from = account_by_id(conn, from_user)?;
            let to = account_by_username(conn, &to_username)?;
            if from.id == to.id {
                return Err(BanterError::CannotFriendSelf);
            }
            if are_friends(conn, from.id, to.id)? {
                return Err(BanterError::AlreadyFriends);
            }
            if let Some(existing) = pending_between(conn, from.id, to.id)? {
                if existing.from_id == from.id {
                    return Err(BanterError::RequestExists);
                }
                // The other person already asked. Accepting that request completes the friendship.
                accept_request(conn, from.id, existing.id)?;
                return Ok(existing);
            }
            conn.execute(
                "INSERT INTO friend_requests (from_user, to_user, created_at) VALUES (?1, ?2, ?3)",
                params![from.id, to.id, now_ms()],
            )
            .map_err(db_err)?;
            Ok(FriendRequest {
                id: conn.last_insert_rowid(),
                from_id: from.id,
                from_username: from.username,
                to_id: to.id,
                to_username: to.username,
            })
        })
    }

    pub fn incoming_requests(&self, user_id: i64) -> Result<Vec<FriendRequest>, BanterError> {
        self.request_list(user_id, true)
    }

    pub fn outgoing_requests(&self, user_id: i64) -> Result<Vec<FriendRequest>, BanterError> {
        self.request_list(user_id, false)
    }

    fn request_list(&self, user_id: i64, incoming: bool) -> Result<Vec<FriendRequest>, BanterError> {
        let column = if incoming { "r.to_user" } else { "r.from_user" };
        let sql = format!(
            "SELECT r.id, r.from_user, fu.username, r.to_user, tu.username
             FROM friend_requests r
             JOIN users fu ON fu.id = r.from_user
             JOIN users tu ON tu.id = r.to_user
             WHERE {column} = ?1
             ORDER BY r.id"
        );
        self.with(|conn| {
            let mut stmt = conn.prepare(&sql).map_err(db_err)?;
            let rows = stmt
                .query_map([user_id], map_request)
                .map_err(db_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db_err)?;
            Ok(rows)
        })
    }

    pub fn accept_friend_request(&self, user_id: i64, request_id: i64) -> Result<(), BanterError> {
        self.with_mut(|conn| accept_request(conn, user_id, request_id))
    }

    pub fn decline_friend_request(&self, user_id: i64, request_id: i64) -> Result<(), BanterError> {
        self.with_mut(|conn| {
            let request = request_by_id(conn, request_id)?;
            if request.to_id != user_id {
                return Err(BanterError::NotPending);
            }
            conn.execute("DELETE FROM friend_requests WHERE id = ?1", [request_id]).map_err(db_err)?;
            Ok(())
        })
    }

    pub fn friends(&self, user_id: i64) -> Result<Vec<Friend>, BanterError> {
        self.with(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT u.id, u.username, u.display_name, u.avatar_file, u.avatar_rev
                     FROM friendships f
                     JOIN users u ON u.id = CASE WHEN f.user_a = ?1 THEN f.user_b ELSE f.user_a END
                     WHERE f.user_a = ?1 OR f.user_b = ?1
                     ORDER BY u.username",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map([user_id], |row| {
                    Ok(Friend {
                        id: row.get(0)?,
                        username: row.get(1)?,
                        display_name: row.get(2)?,
                        has_avatar: row.get::<_, Option<String>>(3)?.is_some(),
                        avatar_rev: row.get(4)?,
                    })
                })
                .map_err(db_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db_err)?;
            Ok(rows)
        })
    }

    pub fn create_server(&self, owner_id: i64, name: &str) -> Result<ServerHome, BanterError> {
        let name = clean_label(name)?;
        self.account(owner_id)?;
        let server_id = self.with_mut(|conn| {
            let tx = conn.transaction().map_err(db_err)?;
            tx.execute(
                "INSERT INTO servers (name, owner_id, created_at) VALUES (?1, ?2, ?3)",
                params![name, owner_id, now_ms()],
            )
            .map_err(db_err)?;
            let server_id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO memberships (server_id, user_id, nickname) VALUES (?1, ?2, NULL)",
                params![server_id, owner_id],
            )
            .map_err(db_err)?;
            insert_category(&tx, server_id, "Text Channels", 0)?;
            let text_category = tx.last_insert_rowid();
            insert_channel(&tx, server_id, text_category, "general", ChannelKind::Text, 0)?;
            insert_category(&tx, server_id, "Voice Channels", 1)?;
            let voice_category = tx.last_insert_rowid();
            insert_channel(&tx, server_id, voice_category, "General", ChannelKind::Voice, 0)?;
            tx.commit().map_err(db_err)?;
            Ok(server_id)
        })?;
        self.server_home(owner_id, server_id)
    }

    pub fn create_invite(&self, user_id: i64, server_id: i64) -> Result<String, BanterError> {
        self.require_member(user_id, server_id)?;
        self.with_mut(|conn| {
            for _ in 0..8 {
                let code = invite_code();
                let inserted = conn
                    .execute(
                        "INSERT INTO invites (code, server_id, created_by, created_at) VALUES (?1, ?2, ?3, ?4)",
                        params![code, server_id, user_id, now_ms()],
                    )
                    .optional_insert();
                if inserted.is_ok() {
                    return Ok(code);
                }
            }
            Err(BanterError::Storage("could not allocate an invite code".into()))
        })
    }

    pub fn join_invite(&self, user_id: i64, code: &str) -> Result<ServerHome, BanterError> {
        self.account(user_id)?;
        let server_id = self.with_mut(|conn| {
            let server_id: Option<i64> = conn
                .query_row("SELECT server_id FROM invites WHERE code = ?1", [code], |row| row.get(0))
                .optional()
                .map_err(db_err)?;
            let Some(server_id) = server_id else {
                return Err(BanterError::InvalidInvite);
            };
            if is_member(conn, user_id, server_id)? {
                return Err(BanterError::AlreadyMember);
            }
            conn.execute(
                "INSERT INTO memberships (server_id, user_id, nickname) VALUES (?1, ?2, NULL)",
                params![server_id, user_id],
            )
            .map_err(db_err)?;
            Ok(server_id)
        })?;
        self.server_home(user_id, server_id)
    }

    pub fn servers(&self, user_id: i64) -> Result<Vec<ServerSummary>, BanterError> {
        self.with(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT s.id, s.name, s.owner_id
                     FROM servers s
                     JOIN memberships m ON m.server_id = s.id
                     WHERE m.user_id = ?1
                     ORDER BY s.id",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map([user_id], map_server)
                .map_err(db_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db_err)?;
            Ok(rows)
        })
    }

    pub fn members(&self, user_id: i64, server_id: i64) -> Result<Vec<Member>, BanterError> {
        self.require_member(user_id, server_id)?;
        self.with(|conn| members_of(conn, server_id))
    }

    pub fn set_nickname(&self, user_id: i64, server_id: i64, nickname: Option<&str>) -> Result<(), BanterError> {
        self.require_member(user_id, server_id)?;
        let nickname = match nickname {
            Some(value) => clean_nickname(value)?,
            None => None,
        };
        self.with_mut(|conn| {
            conn.execute(
                "UPDATE memberships SET nickname = ?1 WHERE server_id = ?2 AND user_id = ?3",
                params![nickname, server_id, user_id],
            )
            .map_err(db_err)?;
            Ok(())
        })
    }

    pub fn create_category(&self, user_id: i64, server_id: i64, name: &str) -> Result<Category, BanterError> {
        let name = clean_label(name)?;
        self.require_owner(user_id, server_id)?;
        self.with_mut(|conn| {
            let position: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(position) + 1, 0) FROM categories WHERE server_id = ?1",
                    [server_id],
                    |row| row.get(0),
                )
                .map_err(db_err)?;
            insert_category(conn, server_id, &name, position)?;
            Ok(Category {
                id: conn.last_insert_rowid(),
                server_id,
                name,
                position,
            })
        })
    }

    pub fn create_channel(
        &self,
        user_id: i64,
        server_id: i64,
        category_id: i64,
        name: &str,
        kind: ChannelKind,
    ) -> Result<Channel, BanterError> {
        let name = clean_label(name)?;
        self.require_owner(user_id, server_id)?;
        self.with_mut(|conn| {
            let category_server: Option<i64> = conn
                .query_row("SELECT server_id FROM categories WHERE id = ?1", [category_id], |row| row.get(0))
                .optional()
                .map_err(db_err)?;
            if category_server != Some(server_id) {
                return Err(BanterError::NotFound);
            }
            let position: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(position) + 1, 0) FROM channels WHERE category_id = ?1",
                    [category_id],
                    |row| row.get(0),
                )
                .map_err(db_err)?;
            insert_channel(conn, server_id, category_id, &name, kind, position)?;
            Ok(Channel {
                id: conn.last_insert_rowid(),
                server_id,
                category_id,
                name,
                kind,
                position,
            })
        })
    }

    pub fn layout(&self, user_id: i64, server_id: i64) -> Result<Vec<CategoryLayout>, BanterError> {
        self.require_member(user_id, server_id)?;
        self.with(|conn| layout_of(conn, server_id))
    }

    pub fn send_message(
        &self,
        user_id: i64,
        target: ChatTarget,
        body: &str,
        reply_to: Option<i64>,
        files: &[(&str, &[u8])],
    ) -> Result<Message, BanterError> {
        let body = body.trim().to_string();
        if body.chars().count() > MAX_MESSAGE_CHARS {
            return Err(BanterError::MessageTooLong);
        }
        if body.is_empty() && files.is_empty() {
            return Err(BanterError::EmptyMessage);
        }
        for (_, bytes) in files {
            if bytes.is_empty() || bytes.len() > MAX_ATTACHMENT_BYTES {
                return Err(BanterError::FileTooLarge);
            }
        }
        let files_dir = self.root.join("files");
        let mut written = Vec::new();
        let result = self.with_mut(|conn| {
            let tx = conn.transaction().map_err(db_err)?;
            require_text_access(&tx, user_id, target)?;
            if let Some(reply_id) = reply_to {
                if !reply_in_target(&tx, reply_id, target)? {
                    return Err(BanterError::ReplyTargetMissing);
                }
            }
            let (channel_id, dm_id) = target_ids(target);
            tx.execute(
                "INSERT INTO messages (channel_id, dm_id, author_id, body, reply_to, created_at, edited_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
                params![channel_id, dm_id, user_id, body, reply_to, now_ms()],
            )
            .map_err(db_err)?;
            let message_id = tx.last_insert_rowid();
            for (name, bytes) in files {
                let filename = safe_filename(name);
                tx.execute(
                    "INSERT INTO attachments (message_id, filename, byte_size) VALUES (?1, ?2, ?3)",
                    params![message_id, filename, bytes.len() as i64],
                )
                .map_err(db_err)?;
                let attachment_id = tx.last_insert_rowid();
                let path = files_dir.join(attachment_id.to_string());
                if let Err(err) = fs::write(&path, bytes) {
                    for path in &written {
                        let _ = fs::remove_file(path);
                    }
                    let _ = fs::remove_file(&path);
                    return Err(BanterError::Storage(err.to_string()));
                }
                written.push(path);
            }
            replace_mentions(&tx, message_id, user_id, &body, target)?;
            let message = load_message(&tx, message_id)?;
            tx.commit().map_err(db_err)?;
            Ok(message)
        });
        if result.is_err() {
            for path in written {
                let _ = fs::remove_file(path);
            }
        }
        result
    }

    pub fn edit_message(&self, user_id: i64, message_id: i64, body: &str) -> Result<Message, BanterError> {
        let body = body.trim().to_string();
        if body.is_empty() {
            return Err(BanterError::EmptyMessage);
        }
        if body.chars().count() > MAX_MESSAGE_CHARS {
            return Err(BanterError::MessageTooLong);
        }
        self.with_mut(|conn| {
            let existing = load_message(conn, message_id)?;
            if existing.author_id != user_id {
                return Err(BanterError::NotAuthor);
            }
            let target = message_target(conn, message_id)?;
            conn.execute(
                "UPDATE messages SET body = ?1, edited_at = ?2 WHERE id = ?3",
                params![body, now_ms(), message_id],
            )
            .map_err(db_err)?;
            replace_mentions(conn, message_id, user_id, &body, target)?;
            load_message(conn, message_id)
        })
    }

    pub fn delete_message(&self, user_id: i64, message_id: i64) -> Result<(), BanterError> {
        let files_dir = self.root.join("files");
        let paths = self.with_mut(|conn| {
            let existing = load_message(conn, message_id)?;
            if existing.author_id != user_id {
                return Err(BanterError::NotAuthor);
            }
            let paths: Vec<PathBuf> = existing
                .attachments
                .iter()
                .map(|attachment| files_dir.join(attachment.id.to_string()))
                .collect();
            let tx = conn.transaction().map_err(db_err)?;
            tx.execute("UPDATE messages SET reply_to = NULL WHERE reply_to = ?1", [message_id]).map_err(db_err)?;
            tx.execute("DELETE FROM messages WHERE id = ?1", [message_id]).map_err(db_err)?;
            tx.commit().map_err(db_err)?;
            Ok(paths)
        })?;
        for path in paths {
            let _ = fs::remove_file(path);
        }
        Ok(())
    }

    pub fn messages(&self, user_id: i64, target: ChatTarget, limit: u32) -> Result<Vec<Message>, BanterError> {
        self.message_query(user_id, target, limit, None)
    }

    pub fn search_messages(&self, user_id: i64, target: ChatTarget, query: &str) -> Result<Vec<Message>, BanterError> {
        let query = query.trim();
        if query.is_empty() {
            return self.messages(user_id, target, 100);
        }
        self.message_query(user_id, target, 200, Some(query))
    }

    fn message_query(
        &self,
        user_id: i64,
        target: ChatTarget,
        limit: u32,
        query: Option<&str>,
    ) -> Result<Vec<Message>, BanterError> {
        let limit = limit.clamp(1, 1000) as i64;
        self.with(|conn| {
            require_text_access(conn, user_id, target)?;
            let (column, id) = match target {
                ChatTarget::Channel(id) => ("m.channel_id", id),
                ChatTarget::Direct(id) => ("m.dm_id", id),
            };
            let (extra, pattern) = match query {
                Some(query) => ("AND lower(m.body) LIKE ?2 ESCAPE '\\'", Some(like_pattern(&query.to_lowercase()))),
                None => ("", None),
            };
            let sql = format!(
                "SELECT m.id FROM messages m
                 WHERE {column} = ?1 {extra}
                 ORDER BY m.id DESC
                 LIMIT {limit}"
            );
            let mut stmt = conn.prepare(&sql).map_err(db_err)?;
            let ids = match &pattern {
                Some(pattern) => stmt
                    .query_map(params![id, pattern], |row| row.get::<_, i64>(0))
                    .map_err(db_err)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(db_err)?,
                None => stmt
                    .query_map(params![id], |row| row.get::<_, i64>(0))
                    .map_err(db_err)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(db_err)?,
            };
            let mut messages = Vec::with_capacity(ids.len());
            for id in ids.into_iter().rev() {
                messages.push(load_message(conn, id)?);
            }
            Ok(messages)
        })
    }

    pub fn read_attachment(&self, user_id: i64, attachment_id: i64) -> Result<(String, Vec<u8>), BanterError> {
        let (filename, message_id) = self.with(|conn| {
            conn.query_row(
                "SELECT filename, message_id FROM attachments WHERE id = ?1",
                [attachment_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(db_err)?
            .ok_or(BanterError::NotFound)
        })?;
        let target = self.with(|conn| message_target(conn, message_id))?;
        self.with(|conn| require_text_access(conn, user_id, target))?;
        let bytes = fs::read(self.root.join("files").join(attachment_id.to_string()))
            .map_err(|err| BanterError::Storage(err.to_string()))?;
        Ok((filename, bytes))
    }

    pub fn open_dm(&self, user_id: i64, peer_id: i64) -> Result<DirectChat, BanterError> {
        if user_id == peer_id {
            return Err(BanterError::NotFriend);
        }
        self.with_mut(|conn| {
            if !are_friends(conn, user_id, peer_id)? {
                return Err(BanterError::NotFriend);
            }
            let (low, high) = ordered_pair(user_id, peer_id);
            let existing: Option<i64> = conn
                .query_row(
                    "SELECT id FROM direct_messages WHERE user_a = ?1 AND user_b = ?2",
                    params![low, high],
                    |row| row.get(0),
                )
                .optional()
                .map_err(db_err)?;
            let id = if let Some(id) = existing {
                id
            } else {
                conn.execute(
                    "INSERT INTO direct_messages (user_a, user_b, created_at) VALUES (?1, ?2, ?3)",
                    params![low, high, now_ms()],
                )
                .map_err(db_err)?;
                conn.last_insert_rowid()
            };
            direct_chat(conn, user_id, id)
        })
    }

    pub fn direct_messages(&self, user_id: i64) -> Result<Vec<DirectChat>, BanterError> {
        self.with(|conn| {
            let mut stmt = conn
                .prepare("SELECT id FROM direct_messages WHERE user_a = ?1 OR user_b = ?1 ORDER BY id")
                .map_err(db_err)?;
            let ids = stmt
                .query_map([user_id], |row| row.get::<_, i64>(0))
                .map_err(db_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db_err)?;
            let mut chats = Vec::new();
            for id in ids {
                chats.push(direct_chat(conn, user_id, id)?);
            }
            Ok(chats)
        })
    }

    pub fn unread_mentions(&self, user_id: i64) -> Result<Vec<Mention>, BanterError> {
        self.with(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT mn.id, mn.message_id, m.channel_id, m.dm_id, u.username, m.body
                     FROM mentions mn
                     JOIN messages m ON m.id = mn.message_id
                     JOIN users u ON u.id = m.author_id
                     WHERE mn.user_id = ?1 AND mn.read_at IS NULL
                     ORDER BY mn.id",
                )
                .map_err(db_err)?;
            let rows = stmt
                .query_map([user_id], |row| {
                    Ok(Mention {
                        id: row.get(0)?,
                        message_id: row.get(1)?,
                        channel_id: row.get(2)?,
                        dm_id: row.get(3)?,
                        from_username: row.get(4)?,
                        body: row.get(5)?,
                    })
                })
                .map_err(db_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db_err)?;
            Ok(rows)
        })
    }

    pub fn mark_mention_read(&self, user_id: i64, mention_id: i64) -> Result<(), BanterError> {
        self.with_mut(|conn| {
            let changed = conn
                .execute(
                    "UPDATE mentions SET read_at = ?1 WHERE id = ?2 AND user_id = ?3",
                    params![now_ms(), mention_id, user_id],
                )
                .map_err(db_err)?;
            if changed == 0 {
                return Err(BanterError::NotFound);
            }
            Ok(())
        })
    }

    pub(crate) fn channel_access(&self, user_id: i64, channel_id: i64) -> Result<Channel, BanterError> {
        self.with(|conn| {
            let channel = channel_by_id(conn, channel_id)?;
            if !is_member(conn, user_id, channel.server_id)? {
                return Err(BanterError::NotMember);
            }
            Ok(channel)
        })
    }

    pub(crate) fn dm_participants(&self, user_id: i64, dm_id: i64) -> Result<(i64, i64), BanterError> {
        self.with(|conn| {
            let row: Option<(i64, i64)> = conn
                .query_row(
                    "SELECT user_a, user_b FROM direct_messages WHERE id = ?1",
                    [dm_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(db_err)?;
            let Some((low, high)) = row else {
                return Err(BanterError::NotFound);
            };
            if user_id == low {
                Ok((low, high))
            } else if user_id == high {
                Ok((high, low))
            } else {
                Err(BanterError::NotMember)
            }
        })
    }

    pub(crate) fn user_by_username(&self, username: &str) -> Result<Account, BanterError> {
        let username = normalize_username(username)?;
        self.with(|conn| account_by_username(conn, &username))
    }

    pub(crate) fn target_user_ids(&self, target: ChatTarget) -> Result<Vec<i64>, BanterError> {
        self.with(|conn| visible_user_ids(conn, target))
    }

    pub(crate) fn message_target(&self, message_id: i64) -> Result<ChatTarget, BanterError> {
        self.with(|conn| message_target(conn, message_id))
    }

    pub(crate) fn related_user_ids(&self, user_id: i64) -> Result<Vec<i64>, BanterError> {
        self.with(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT user_b FROM friendships WHERE user_a = ?1
                     UNION SELECT user_a FROM friendships WHERE user_b = ?1
                     UNION SELECT m2.user_id FROM memberships m1
                     JOIN memberships m2 ON m1.server_id = m2.server_id
                     WHERE m1.user_id = ?1",
                )
                .map_err(db_err)?;
            let mut ids = stmt
                .query_map([user_id], |row| row.get::<_, i64>(0))
                .map_err(db_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db_err)?;
            if !ids.contains(&user_id) {
                ids.push(user_id);
            }
            Ok(ids)
        })
    }

    fn server_home(&self, user_id: i64, server_id: i64) -> Result<ServerHome, BanterError> {
        self.with(|conn| {
            let server = conn
                .query_row(
                    "SELECT id, name, owner_id FROM servers WHERE id = ?1",
                    [server_id],
                    map_server,
                )
                .map_err(db_err)?;
            let _ = user_id;
            Ok(ServerHome {
                server,
                categories: layout_of(conn, server_id)?,
            })
        })
    }

    fn require_member(&self, user_id: i64, server_id: i64) -> Result<(), BanterError> {
        self.with(|conn| {
            if is_member(conn, user_id, server_id)? {
                Ok(())
            } else {
                Err(BanterError::NotMember)
            }
        })
    }

    fn require_owner(&self, user_id: i64, server_id: i64) -> Result<(), BanterError> {
        self.with(|conn| {
            if !is_member(conn, user_id, server_id)? {
                return Err(BanterError::NotMember);
            }
            let owner: i64 = conn
                .query_row("SELECT owner_id FROM servers WHERE id = ?1", [server_id], |row| row.get(0))
                .map_err(db_err)?;
            if owner == user_id {
                Ok(())
            } else {
                Err(BanterError::NotOwner)
            }
        })
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> Result<T, BanterError>) -> Result<T, BanterError> {
        let conn = self.conn.lock().expect("sqlite lock");
        f(&conn)
    }

    fn with_mut<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T, BanterError>) -> Result<T, BanterError> {
        let mut conn = self.conn.lock().expect("sqlite lock");
        f(&mut conn)
    }
}

fn account_by_id(conn: &Connection, id: i64) -> Result<Account, BanterError> {
    conn.query_row(
        "SELECT id, username, display_name, avatar_file, avatar_rev FROM users WHERE id = ?1",
        [id],
        map_account,
    )
    .optional()
    .map_err(db_err)?
    .ok_or(BanterError::NotFound)
}

fn account_by_username(conn: &Connection, username: &str) -> Result<Account, BanterError> {
    conn.query_row(
        "SELECT id, username, display_name, avatar_file, avatar_rev FROM users WHERE username = ?1",
        [username],
        map_account,
    )
    .optional()
    .map_err(db_err)?
    .ok_or(BanterError::NotFound)
}

fn map_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    Ok(Account {
        id: row.get(0)?,
        username: row.get(1)?,
        display_name: row.get(2)?,
        has_avatar: row.get::<_, Option<String>>(3)?.is_some(),
        avatar_rev: row.get(4)?,
    })
}

fn map_server(row: &rusqlite::Row<'_>) -> rusqlite::Result<ServerSummary> {
    Ok(ServerSummary {
        id: row.get(0)?,
        name: row.get(1)?,
        owner_id: row.get(2)?,
    })
}

fn map_request(row: &rusqlite::Row<'_>) -> rusqlite::Result<FriendRequest> {
    Ok(FriendRequest {
        id: row.get(0)?,
        from_id: row.get(1)?,
        from_username: row.get(2)?,
        to_id: row.get(3)?,
        to_username: row.get(4)?,
    })
}

fn request_by_id(conn: &Connection, id: i64) -> Result<FriendRequest, BanterError> {
    conn.query_row(
        "SELECT r.id, r.from_user, fu.username, r.to_user, tu.username
         FROM friend_requests r
         JOIN users fu ON fu.id = r.from_user
         JOIN users tu ON tu.id = r.to_user
         WHERE r.id = ?1",
        [id],
        map_request,
    )
    .optional()
    .map_err(db_err)?
    .ok_or(BanterError::NotPending)
}

fn pending_between(conn: &Connection, a: i64, b: i64) -> Result<Option<FriendRequest>, BanterError> {
    conn.query_row(
        "SELECT r.id, r.from_user, fu.username, r.to_user, tu.username
         FROM friend_requests r
         JOIN users fu ON fu.id = r.from_user
         JOIN users tu ON tu.id = r.to_user
         WHERE (r.from_user = ?1 AND r.to_user = ?2) OR (r.from_user = ?2 AND r.to_user = ?1)",
        params![a, b],
        map_request,
    )
    .optional()
    .map_err(db_err)
}

fn accept_request(conn: &Connection, user_id: i64, request_id: i64) -> Result<(), BanterError> {
    let request = request_by_id(conn, request_id)?;
    if request.to_id != user_id {
        return Err(BanterError::NotPending);
    }
    let (low, high) = ordered_pair(request.from_id, request.to_id);
    conn.execute(
        "INSERT INTO friendships (user_a, user_b, created_at) VALUES (?1, ?2, ?3)",
        params![low, high, now_ms()],
    )
    .map_err(db_err)?;
    conn.execute("DELETE FROM friend_requests WHERE id = ?1", [request_id]).map_err(db_err)?;
    Ok(())
}

fn are_friends(conn: &Connection, a: i64, b: i64) -> Result<bool, BanterError> {
    let (low, high) = ordered_pair(a, b);
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM friendships WHERE user_a = ?1 AND user_b = ?2",
            params![low, high],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_err)?;
    Ok(found.is_some())
}

fn ordered_pair(a: i64, b: i64) -> (i64, i64) {
    if a < b { (a, b) } else { (b, a) }
}

fn is_member(conn: &Connection, user_id: i64, server_id: i64) -> Result<bool, BanterError> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM memberships WHERE user_id = ?1 AND server_id = ?2",
            params![user_id, server_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_err)?;
    Ok(found.is_some())
}

fn members_of(conn: &Connection, server_id: i64) -> Result<Vec<Member>, BanterError> {
    let owner: i64 = conn
        .query_row("SELECT owner_id FROM servers WHERE id = ?1", [server_id], |row| row.get(0))
        .map_err(db_err)?;
    let mut stmt = conn
        .prepare(
            "SELECT u.id, u.username, u.display_name, m.nickname, u.avatar_file, u.avatar_rev
             FROM memberships m
             JOIN users u ON u.id = m.user_id
             WHERE m.server_id = ?1
             ORDER BY u.username",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([server_id], |row| {
            let user_id: i64 = row.get(0)?;
            Ok(Member {
                user_id,
                username: row.get(1)?,
                display_name: row.get(2)?,
                nickname: row.get(3)?,
                is_owner: user_id == owner,
                has_avatar: row.get::<_, Option<String>>(4)?.is_some(),
                avatar_rev: row.get(5)?,
            })
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

fn insert_category(conn: &Connection, server_id: i64, name: &str, position: i64) -> Result<(), BanterError> {
    conn.execute(
        "INSERT INTO categories (server_id, name, position) VALUES (?1, ?2, ?3)",
        params![server_id, name, position],
    )
    .map_err(db_err)?;
    Ok(())
}

fn insert_channel(
    conn: &Connection,
    server_id: i64,
    category_id: i64,
    name: &str,
    kind: ChannelKind,
    position: i64,
) -> Result<(), BanterError> {
    conn.execute(
        "INSERT INTO channels (server_id, category_id, name, kind, position) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![server_id, category_id, name, kind.as_str(), position],
    )
    .map_err(db_err)?;
    Ok(())
}

fn layout_of(conn: &Connection, server_id: i64) -> Result<Vec<CategoryLayout>, BanterError> {
    let mut categories = conn
        .prepare("SELECT id, server_id, name, position FROM categories WHERE server_id = ?1 ORDER BY position, id")
        .map_err(db_err)?;
    let categories = categories
        .query_map([server_id], |row| {
            Ok(Category {
                id: row.get(0)?,
                server_id: row.get(1)?,
                name: row.get(2)?,
                position: row.get(3)?,
            })
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    let mut layouts = Vec::new();
    for category in categories {
        let mut channels = conn
            .prepare(
                "SELECT id, server_id, category_id, name, kind, position
                 FROM channels WHERE category_id = ?1 ORDER BY position, id",
            )
            .map_err(db_err)?;
        let channels = channels
            .query_map([category.id], |row| {
                Ok(Channel {
                    id: row.get(0)?,
                    server_id: row.get(1)?,
                    category_id: row.get(2)?,
                    name: row.get(3)?,
                    kind: ChannelKind::parse(&row.get::<_, String>(4)?).unwrap_or(ChannelKind::Text),
                    position: row.get(5)?,
                })
            })
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        layouts.push(CategoryLayout { category, channels });
    }
    Ok(layouts)
}

fn channel_by_id(conn: &Connection, id: i64) -> Result<Channel, BanterError> {
    conn.query_row(
        "SELECT id, server_id, category_id, name, kind, position FROM channels WHERE id = ?1",
        [id],
        |row| {
            Ok(Channel {
                id: row.get(0)?,
                server_id: row.get(1)?,
                category_id: row.get(2)?,
                name: row.get(3)?,
                kind: ChannelKind::parse(&row.get::<_, String>(4)?).unwrap_or(ChannelKind::Text),
                position: row.get(5)?,
            })
        },
    )
    .optional()
    .map_err(db_err)?
    .ok_or(BanterError::NotFound)
}

fn require_text_access(conn: &Connection, user_id: i64, target: ChatTarget) -> Result<(), BanterError> {
    match target {
        ChatTarget::Channel(channel_id) => {
            let channel = channel_by_id(conn, channel_id)?;
            if !is_member(conn, user_id, channel.server_id)? {
                return Err(BanterError::NotMember);
            }
            if channel.kind != ChannelKind::Text {
                return Err(BanterError::NotTextChannel);
            }
            Ok(())
        }
        ChatTarget::Direct(dm_id) => {
            let row: Option<(i64, i64)> = conn
                .query_row(
                    "SELECT user_a, user_b FROM direct_messages WHERE id = ?1",
                    [dm_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(db_err)?;
            let Some((low, high)) = row else {
                return Err(BanterError::NotFound);
            };
            if user_id == low || user_id == high {
                Ok(())
            } else {
                Err(BanterError::NotMember)
            }
        }
    }
}

fn target_ids(target: ChatTarget) -> (Option<i64>, Option<i64>) {
    match target {
        ChatTarget::Channel(id) => (Some(id), None),
        ChatTarget::Direct(id) => (None, Some(id)),
    }
}

fn reply_in_target(conn: &Connection, reply_id: i64, target: ChatTarget) -> Result<bool, BanterError> {
    let found = message_target(conn, reply_id);
    match found {
        Ok(existing) => Ok(existing == target),
        Err(BanterError::NotFound) => Ok(false),
        Err(err) => Err(err),
    }
}

fn message_target(conn: &Connection, message_id: i64) -> Result<ChatTarget, BanterError> {
    let row: Option<(Option<i64>, Option<i64>)> = conn
        .query_row(
            "SELECT channel_id, dm_id FROM messages WHERE id = ?1",
            [message_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(db_err)?;
    match row {
        Some((Some(channel), _)) => Ok(ChatTarget::Channel(channel)),
        Some((_, Some(dm))) => Ok(ChatTarget::Direct(dm)),
        _ => Err(BanterError::NotFound),
    }
}

fn load_message(conn: &Connection, message_id: i64) -> Result<Message, BanterError> {
    let message = conn
        .query_row(
            "SELECT m.id, m.author_id, u.username, u.display_name, mem.nickname,
                    m.body, m.reply_to, m.created_at, m.edited_at
             FROM messages m
             JOIN users u ON u.id = m.author_id
             LEFT JOIN channels c ON c.id = m.channel_id
             LEFT JOIN memberships mem ON mem.user_id = m.author_id AND mem.server_id = c.server_id
             WHERE m.id = ?1",
            [message_id],
            |row| {
                Ok(Message {
                    id: row.get(0)?,
                    author_id: row.get(1)?,
                    author_username: row.get(2)?,
                    author_display_name: row.get(3)?,
                    author_nickname: row.get(4)?,
                    body: row.get(5)?,
                    reply_to: row.get(6)?,
                    created_at: row.get(7)?,
                    edited_at: row.get(8)?,
                    attachments: Vec::new(),
                })
            },
        )
        .optional()
        .map_err(db_err)?
        .ok_or(BanterError::NotFound)?;
    let mut stmt = conn
        .prepare("SELECT id, filename, byte_size FROM attachments WHERE message_id = ?1 ORDER BY id")
        .map_err(db_err)?;
    let attachments = stmt
        .query_map([message_id], |row| {
            Ok(Attachment {
                id: row.get(0)?,
                filename: row.get(1)?,
                byte_size: row.get(2)?,
            })
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(Message { attachments, ..message })
}

fn replace_mentions(
    conn: &Connection,
    message_id: i64,
    author_id: i64,
    body: &str,
    target: ChatTarget,
) -> Result<(), BanterError> {
    conn.execute("DELETE FROM mentions WHERE message_id = ?1", [message_id]).map_err(db_err)?;
    let visible = visible_users(conn, target)?;
    for token in mention_usernames(body) {
        let Some((user_id, _)) = visible.iter().find(|(_, username)| username == &token) else {
            continue;
        };
        if *user_id == author_id {
            continue;
        }
        conn.execute(
            "INSERT INTO mentions (message_id, user_id, read_at) VALUES (?1, ?2, NULL)",
            params![message_id, user_id],
        )
        .map_err(db_err)?;
    }
    Ok(())
}

fn visible_users(conn: &Connection, target: ChatTarget) -> Result<Vec<(i64, String)>, BanterError> {
    match target {
        ChatTarget::Channel(channel_id) => {
            let server_id: i64 = conn
                .query_row("SELECT server_id FROM channels WHERE id = ?1", [channel_id], |row| row.get(0))
                .map_err(db_err)?;
            let mut stmt = conn
                .prepare(
                    "SELECT u.id, u.username FROM memberships m
                     JOIN users u ON u.id = m.user_id
                     WHERE m.server_id = ?1",
                )
                .map_err(db_err)?;
            let users = stmt
                .query_map([server_id], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(db_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db_err)?;
            Ok(users)
        }
        ChatTarget::Direct(dm_id) => {
            let (low, high): (i64, i64) = conn
                .query_row(
                    "SELECT user_a, user_b FROM direct_messages WHERE id = ?1",
                    [dm_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(db_err)?;
            let mut users = Vec::new();
            for id in [low, high] {
                let account = account_by_id(conn, id)?;
                users.push((account.id, account.username));
            }
            Ok(users)
        }
    }
}

fn visible_user_ids(conn: &Connection, target: ChatTarget) -> Result<Vec<i64>, BanterError> {
    Ok(visible_users(conn, target)?.into_iter().map(|(id, _)| id).collect())
}

fn direct_chat(conn: &Connection, viewer: i64, dm_id: i64) -> Result<DirectChat, BanterError> {
    let (low, high): (i64, i64) = conn
        .query_row(
            "SELECT user_a, user_b FROM direct_messages WHERE id = ?1",
            [dm_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(db_err)?;
    let peer_id = if viewer == low { high } else { low };
    let peer = account_by_id(conn, peer_id)?;
    Ok(DirectChat {
        id: dm_id,
        peer_id,
        peer_username: peer.username,
        peer_display_name: peer.display_name,
        peer_has_avatar: peer.has_avatar,
        peer_avatar_rev: peer.avatar_rev,
    })
}

fn mention_usernames(body: &str) -> Vec<String> {
    let chars: Vec<(usize, char)> = body.char_indices().collect();
    let mut found = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index].1 == '@' {
            let boundary = index == 0 || !is_username_char(chars[index - 1].1);
            if boundary {
                let mut end = index + 1;
                while end < chars.len() && is_username_char(chars[end].1) {
                    end += 1;
                }
                if end > index + 1 {
                    let start = chars[index + 1].0;
                    let stop = chars.get(end).map(|(offset, _)| *offset).unwrap_or(body.len());
                    let token = body[start..stop].to_lowercase();
                    if username_ok(&token) && !found.contains(&token) {
                        found.push(token);
                    }
                    index = end;
                    continue;
                }
            }
        }
        index += 1;
    }
    found
}

fn is_username_char(value: char) -> bool {
    value.is_ascii_lowercase()
        || value.is_ascii_uppercase()
        || value.is_ascii_digit()
        || value == '_'
        || value == '.'
}

fn username_ok(name: &str) -> bool {
    let len = name.chars().count();
    if !(2..=32).contains(&len) {
        return false;
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.')
    {
        return false;
    }
    !name.starts_with('.') && !name.ends_with('.') && !name.contains("..")
}

fn normalize_username(raw: &str) -> Result<String, BanterError> {
    let name = raw.trim().to_lowercase();
    if username_ok(&name) {
        Ok(name)
    } else {
        Err(BanterError::InvalidUsername)
    }
}

fn clean_display(raw: &str) -> Result<String, BanterError> {
    let name = raw.trim();
    let count = name.chars().count();
    if (1..=32).contains(&count) && !name.chars().any(|c| c.is_control()) {
        Ok(name.to_string())
    } else {
        Err(BanterError::InvalidDisplayName)
    }
}

fn clean_label(raw: &str) -> Result<String, BanterError> {
    let name = raw.trim();
    let count = name.chars().count();
    if (1..=100).contains(&count) && !name.chars().any(|c| c.is_control()) {
        Ok(name.to_string())
    } else {
        Err(BanterError::InvalidName)
    }
}

fn clean_nickname(raw: &str) -> Result<Option<String>, BanterError> {
    let name = raw.trim();
    if name.is_empty() {
        return Ok(None);
    }
    if name.chars().count() > 32 || name.chars().any(|c| c.is_control()) {
        return Err(BanterError::InvalidName);
    }
    Ok(Some(name.to_string()))
}

fn check_password(password: &str) -> Result<(), BanterError> {
    let count = password.chars().count();
    if (8..=128).contains(&count) {
        Ok(())
    } else {
        Err(BanterError::InvalidPassword)
    }
}

fn hash_password(password: &str) -> Result<String, BanterError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|err| BanterError::Storage(err.to_string()))
}

fn verify_password(password: &str, hash: &str) -> Result<(), BanterError> {
    let parsed = PasswordHash::new(hash).map_err(|_| BanterError::InvalidCredentials)?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| BanterError::InvalidCredentials)
}

fn safe_filename(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    let clean: String = base.chars().filter(|c| !c.is_control()).take(120).collect();
    if clean.is_empty() { "file".into() } else { clean }
}

fn like_pattern(query: &str) -> String {
    let mut pattern = String::from('%');
    for ch in query.chars() {
        if ch == '%' || ch == '_' || ch == '\\' {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    pattern.push('%');
    pattern
}

fn invite_code() -> String {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut rng = rand::thread_rng();
    (0..10)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::thread_rng().fill(&mut bytes);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn db_err(err: rusqlite::Error) -> BanterError {
    BanterError::Storage(err.to_string())
}

fn constraint(err: rusqlite::Error, unique: BanterError) -> BanterError {
    match err {
        rusqlite::Error::SqliteFailure(code, _)
            if code.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            unique
        }
        other => db_err(other),
    }
}

trait OptionalInsert {
    fn optional_insert(self) -> Result<(), rusqlite::Error>;
}

impl OptionalInsert for Result<usize, rusqlite::Error> {
    fn optional_insert(self) -> Result<(), rusqlite::Error> {
        self.map(|_| ())
    }
}
