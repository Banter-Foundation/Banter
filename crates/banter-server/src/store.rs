use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use rand::Rng;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::error::BanterError;
use crate::model::*;

pub struct Store {
    pool: PgPool,
    root: PathBuf,
}

impl Store {
    pub async fn open(root: &Path, database_url: &str) -> Result<Self, BanterError> {
        fs::create_dir_all(root).map_err(|err| BanterError::Storage(err.to_string()))?;
        fs::create_dir_all(root.join("files")).map_err(|err| BanterError::Storage(err.to_string()))?;
        fs::create_dir_all(root.join("files").join("board")).map_err(|err| BanterError::Storage(err.to_string()))?;
        fs::create_dir_all(root.join("avatars")).map_err(|err| BanterError::Storage(err.to_string()))?;
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await
            .map_err(db_err)?;
        sqlx::raw_sql(include_str!("schema.sql"))
            .execute(&pool)
            .await
            .map_err(db_err)?;
        Ok(Self {
            pool,
            root: root.to_path_buf(),
        })
    }

    pub async fn register(&self, username: &str, display_name: &str, password: &str) -> Result<Account, BanterError> {
        let username = normalize_username(username)?;
        let display_name = clean_display(display_name)?;
        check_password(password)?;
        let hash = hash_password(password)?;
        let now = now_ms();
        let taken: Option<i64> = sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
            .bind(&username)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        if taken.is_some() {
            return Err(BanterError::UsernameTaken);
        }
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO users (username, display_name, password_hash, avatar_file, created_at)
             VALUES ($1, $2, $3, NULL, $4) RETURNING id",
        )
        .bind(&username)
        .bind(&display_name)
        .bind(&hash)
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .map_err(|err| constraint(err, BanterError::UsernameTaken))?;
        account_by_id(&self.pool, id).await
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<Session, BanterError> {
        let username = normalize_username(username)?;
        let row = sqlx::query("SELECT id, password_hash FROM users WHERE username = $1")
            .bind(&username)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        let Some(row) = row else {
            return Err(BanterError::InvalidCredentials);
        };
        let id: i64 = row.get("id");
        let hash: String = row.get("password_hash");
        verify_password(password, &hash)?;
        let token = random_token();
        sqlx::query("INSERT INTO sessions (token, user_id, created_at) VALUES ($1, $2, $3)")
            .bind(&token)
            .bind(id)
            .bind(now_ms())
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(Session {
            token,
            account: self.account(id).await?,
        })
    }

    pub async fn logout(&self, token: &str) -> Result<(), BanterError> {
        sqlx::query("DELETE FROM sessions WHERE token = $1")
            .bind(token)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    pub async fn session_account(&self, token: &str) -> Result<Account, BanterError> {
        let id: Option<i64> = sqlx::query_scalar("SELECT user_id FROM sessions WHERE token = $1")
            .bind(token)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        let Some(id) = id else {
            return Err(BanterError::InvalidCredentials);
        };
        account_by_id(&self.pool, id).await
    }

    pub async fn account(&self, user_id: i64) -> Result<Account, BanterError> {
        account_by_id(&self.pool, user_id).await
    }

    pub async fn set_display_name(&self, user_id: i64, display_name: &str) -> Result<Account, BanterError> {
        let display_name = clean_display(display_name)?;
        let result = sqlx::query("UPDATE users SET display_name = $1 WHERE id = $2")
            .bind(&display_name)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        if result.rows_affected() == 0 {
            return Err(BanterError::NotFound);
        }
        account_by_id(&self.pool, user_id).await
    }

    pub async fn set_avatar(&self, user_id: i64, bytes: &[u8]) -> Result<(), BanterError> {
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
        self.account(user_id).await?;
        let path = self.root.join("avatars").join(format!("{user_id}.png"));
        fs::write(&path, &png).map_err(|err| BanterError::Storage(err.to_string()))?;
        sqlx::query("UPDATE users SET avatar_file = $1, avatar_rev = avatar_rev + 1 WHERE id = $2")
            .bind(format!("{user_id}.png"))
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    pub async fn clear_avatar(&self, user_id: i64) -> Result<(), BanterError> {
        self.account(user_id).await?;
        let path = self.root.join("avatars").join(format!("{user_id}.png"));
        if path.exists() {
            fs::remove_file(&path).map_err(|err| BanterError::Storage(err.to_string()))?;
        }
        sqlx::query("UPDATE users SET avatar_file = NULL, avatar_rev = 0 WHERE id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    pub async fn avatar_bytes(&self, user_id: i64) -> Result<Vec<u8>, BanterError> {
        let file: Option<String> = sqlx::query_scalar("SELECT avatar_file FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?
            .ok_or(BanterError::NotFound)?;
        let name = file.ok_or(BanterError::NotFound)?;
        let path = self.root.join("avatars").join(name);
        fs::read(&path).map_err(|_| BanterError::NotFound)
    }

    pub async fn send_friend_request(&self, from_user: i64, to_username: &str) -> Result<FriendRequest, BanterError> {
        let to_username = normalize_username(to_username)?;
        let from = account_by_id(&self.pool, from_user).await?;
        let to = account_by_username(&self.pool, &to_username).await?;
        if from.id == to.id {
            return Err(BanterError::CannotFriendSelf);
        }
        if are_friends(&self.pool, from.id, to.id).await? {
            return Err(BanterError::AlreadyFriends);
        }
        if let Some(existing) = pending_between(&self.pool, from.id, to.id).await? {
            if existing.from_id == from.id {
                return Err(BanterError::RequestExists);
            }
            accept_request(&self.pool, from.id, existing.id).await?;
            return Ok(existing);
        }
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO friend_requests (from_user, to_user, created_at) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(from.id)
        .bind(to.id)
        .bind(now_ms())
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(FriendRequest {
            id,
            from_id: from.id,
            from_username: from.username,
            to_id: to.id,
            to_username: to.username,
        })
    }

    pub async fn incoming_requests(&self, user_id: i64) -> Result<Vec<FriendRequest>, BanterError> {
        self.request_list(user_id, true).await
    }

    pub async fn outgoing_requests(&self, user_id: i64) -> Result<Vec<FriendRequest>, BanterError> {
        self.request_list(user_id, false).await
    }

    async fn request_list(&self, user_id: i64, incoming: bool) -> Result<Vec<FriendRequest>, BanterError> {
        let column = if incoming { "r.to_user" } else { "r.from_user" };
        let sql = format!(
            "SELECT r.id, r.from_user, fu.username AS from_username, r.to_user, tu.username AS to_username
             FROM friend_requests r
             JOIN users fu ON fu.id = r.from_user
             JOIN users tu ON tu.id = r.to_user
             WHERE {column} = $1
             ORDER BY r.id"
        );
        let rows = sqlx::query(&sql)
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(rows.iter().map(map_request_row).collect())
    }

    pub async fn accept_friend_request(&self, user_id: i64, request_id: i64) -> Result<(), BanterError> {
        accept_request(&self.pool, user_id, request_id).await
    }

    pub async fn decline_friend_request(&self, user_id: i64, request_id: i64) -> Result<(), BanterError> {
        let request = request_by_id(&self.pool, request_id).await?;
        if request.to_id != user_id {
            return Err(BanterError::NotPending);
        }
        sqlx::query("DELETE FROM friend_requests WHERE id = $1")
            .bind(request_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    pub async fn friends(&self, user_id: i64) -> Result<Vec<Friend>, BanterError> {
        let rows = sqlx::query(
            "SELECT u.id, u.username, u.display_name, u.avatar_file, u.avatar_rev
             FROM friendships f
             JOIN users u ON u.id = CASE WHEN f.user_a = $1 THEN f.user_b ELSE f.user_a END
             WHERE f.user_a = $1 OR f.user_b = $1
             ORDER BY u.username",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows
            .iter()
            .map(|row| Friend {
                id: row.get("id"),
                username: row.get("username"),
                display_name: row.get("display_name"),
                has_avatar: row.get::<Option<String>, _>("avatar_file").is_some(),
                avatar_rev: row.get("avatar_rev"),
            })
            .collect())
    }

    pub async fn create_server(&self, owner_id: i64, name: &str) -> Result<ServerHome, BanterError> {
        let name = clean_label(name)?;
        self.account(owner_id).await?;
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let server_id: i64 = sqlx::query_scalar(
            "INSERT INTO servers (name, owner_id, created_at) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(&name)
        .bind(owner_id)
        .bind(now_ms())
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
        sqlx::query("INSERT INTO memberships (server_id, user_id, nickname) VALUES ($1, $2, NULL)")
            .bind(server_id)
            .bind(owner_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        let text_category = insert_category_tx(&mut tx, server_id, "Text Channels", 0).await?;
        insert_channel_tx(&mut tx, server_id, text_category, "general", ChannelKind::Text, 0).await?;
        let voice_category = insert_category_tx(&mut tx, server_id, "Voice Channels", 1).await?;
        insert_channel_tx(&mut tx, server_id, voice_category, "General", ChannelKind::Voice, 0).await?;
        tx.commit().await.map_err(db_err)?;
        self.server_home(owner_id, server_id).await
    }

    pub async fn create_invite(&self, user_id: i64, server_id: i64) -> Result<String, BanterError> {
        self.require_member(user_id, server_id).await?;
        for _ in 0..8 {
            let code = invite_code();
            let result = sqlx::query(
                "INSERT INTO invites (code, server_id, created_by, created_at) VALUES ($1, $2, $3, $4)
                 ON CONFLICT DO NOTHING",
            )
            .bind(&code)
            .bind(server_id)
            .bind(user_id)
            .bind(now_ms())
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
            if result.rows_affected() == 1 {
                return Ok(code);
            }
        }
        Err(BanterError::Storage("could not allocate an invite code".into()))
    }

    pub async fn join_invite(&self, user_id: i64, code: &str) -> Result<ServerHome, BanterError> {
        self.account(user_id).await?;
        let server_id: Option<i64> = sqlx::query_scalar("SELECT server_id FROM invites WHERE code = $1")
            .bind(code)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        let Some(server_id) = server_id else {
            return Err(BanterError::InvalidInvite);
        };
        if is_member(&self.pool, user_id, server_id).await? {
            return Err(BanterError::AlreadyMember);
        }
        sqlx::query("INSERT INTO memberships (server_id, user_id, nickname) VALUES ($1, $2, NULL)")
            .bind(server_id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        self.server_home(user_id, server_id).await
    }

    pub async fn servers(&self, user_id: i64) -> Result<Vec<ServerSummary>, BanterError> {
        let rows = sqlx::query(
            "SELECT s.id, s.name, s.owner_id
             FROM servers s
             JOIN memberships m ON m.server_id = s.id
             WHERE m.user_id = $1
             ORDER BY s.id",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows
            .iter()
            .map(|row| ServerSummary {
                id: row.get("id"),
                name: row.get("name"),
                owner_id: row.get("owner_id"),
            })
            .collect())
    }

    pub async fn members(&self, user_id: i64, server_id: i64) -> Result<Vec<Member>, BanterError> {
        self.require_member(user_id, server_id).await?;
        members_of(&self.pool, server_id).await
    }

    pub async fn set_nickname(&self, user_id: i64, server_id: i64, nickname: Option<&str>) -> Result<(), BanterError> {
        self.require_member(user_id, server_id).await?;
        let nickname = match nickname {
            Some(value) => clean_nickname(value)?,
            None => None,
        };
        sqlx::query("UPDATE memberships SET nickname = $1 WHERE server_id = $2 AND user_id = $3")
            .bind(nickname)
            .bind(server_id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    pub async fn create_category(&self, user_id: i64, server_id: i64, name: &str) -> Result<Category, BanterError> {
        let name = clean_label(name)?;
        self.require_owner(user_id, server_id).await?;
        let position: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM categories WHERE server_id = $1",
        )
        .bind(server_id)
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO categories (server_id, name, position) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(server_id)
        .bind(&name)
        .bind(position)
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(Category {
            id,
            server_id,
            name,
            position,
        })
    }

    pub async fn create_channel(
        &self,
        user_id: i64,
        server_id: i64,
        category_id: i64,
        name: &str,
        kind: ChannelKind,
    ) -> Result<Channel, BanterError> {
        let name = clean_label(name)?;
        self.require_owner(user_id, server_id).await?;
        let category_server: Option<i64> =
            sqlx::query_scalar("SELECT server_id FROM categories WHERE id = $1")
                .bind(category_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(db_err)?;
        if category_server != Some(server_id) {
            return Err(BanterError::NotFound);
        }
        let position: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM channels WHERE category_id = $1",
        )
        .bind(category_id)
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO channels (server_id, category_id, name, kind, position)
             VALUES ($1, $2, $3, $4, $5) RETURNING id",
        )
        .bind(server_id)
        .bind(category_id)
        .bind(&name)
        .bind(kind.as_str())
        .bind(position)
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        if kind == ChannelKind::Board {
            insert_empty_board_scene(&self.pool, id).await?;
        }
        Ok(Channel {
            id,
            server_id,
            category_id,
            name,
            kind,
            position,
        })
    }

    pub async fn board_scene(&self, user_id: i64, channel_id: i64) -> Result<BoardScene, BanterError> {
        require_board_access(&self.pool, user_id, channel_id).await?;
        load_board_scene(&self.pool, channel_id).await
    }

    pub async fn apply_board_op(&self, user_id: i64, channel_id: i64, op: BoardOp) -> Result<BoardScene, BanterError> {
        require_board_access(&self.pool, user_id, channel_id).await?;
        let mut scene = load_board_scene(&self.pool, channel_id).await?;
        if op.base_revision() != scene.revision {
            return Err(BanterError::BoardConflict);
        }
        apply_op_to_scene(&mut scene, op, user_id)?;
        scene.revision += 1;
        save_board_scene(&self.pool, channel_id, &scene).await?;
        Ok(scene)
    }

    pub async fn upload_board_asset(
        &self,
        user_id: i64,
        channel_id: i64,
        filename: &str,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<BoardAsset, BanterError> {
        if bytes.is_empty() || bytes.len() > MAX_BOARD_ASSET_BYTES {
            return Err(BanterError::FileTooLarge);
        }
        let filename = filename.trim();
        if filename.is_empty() || filename.len() > 200 {
            return Err(BanterError::InvalidName);
        }
        let content_type = if content_type.trim().is_empty() {
            "application/octet-stream"
        } else {
            content_type.trim()
        };
        let dir = self.root.join("files").join("board");
        fs::create_dir_all(&dir).map_err(|err| BanterError::Storage(err.to_string()))?;
        require_board_access(&self.pool, user_id, channel_id).await?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO board_assets (channel_id, filename, byte_size, content_type)
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(channel_id)
        .bind(filename)
        .bind(bytes.len() as i64)
        .bind(content_type)
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        let asset = BoardAsset {
            id,
            channel_id,
            filename: filename.to_string(),
            byte_size: bytes.len() as i64,
            content_type: content_type.to_string(),
        };
        fs::write(dir.join(asset.id.to_string()), bytes).map_err(|err| BanterError::Storage(err.to_string()))?;
        Ok(asset)
    }

    pub async fn read_board_asset(&self, user_id: i64, asset_id: i64) -> Result<(BoardAsset, Vec<u8>), BanterError> {
        let row = sqlx::query(
            "SELECT id, channel_id, filename, byte_size, content_type FROM board_assets WHERE id = $1",
        )
        .bind(asset_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?
        .ok_or(BanterError::NotFound)?;
        let asset = BoardAsset {
            id: row.get("id"),
            channel_id: row.get("channel_id"),
            filename: row.get("filename"),
            byte_size: row.get("byte_size"),
            content_type: row.get("content_type"),
        };
        require_board_access(&self.pool, user_id, asset.channel_id).await?;
        let bytes = fs::read(self.root.join("files").join("board").join(asset.id.to_string()))
            .map_err(|err| BanterError::Storage(err.to_string()))?;
        Ok((asset, bytes))
    }

    pub(crate) async fn channel_member_ids(&self, channel_id: i64) -> Result<Vec<i64>, BanterError> {
        let channel = channel_by_id(&self.pool, channel_id).await?;
        visible_user_ids(&self.pool, ChatTarget::Channel(channel.id)).await
    }

    pub async fn layout(&self, user_id: i64, server_id: i64) -> Result<Vec<CategoryLayout>, BanterError> {
        self.require_member(user_id, server_id).await?;
        layout_of(&self.pool, server_id).await
    }

    pub async fn send_message(
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
        let result = async {
            let mut tx = self.pool.begin().await.map_err(db_err)?;
            require_text_access_tx(&mut tx, user_id, target).await?;
            if let Some(reply_id) = reply_to {
                if !reply_in_target_tx(&mut tx, reply_id, target).await? {
                    return Err(BanterError::ReplyTargetMissing);
                }
            }
            let (channel_id, dm_id) = target_ids(target);
            let message_id: i64 = sqlx::query_scalar(
                "INSERT INTO messages (channel_id, dm_id, author_id, body, reply_to, created_at, edited_at)
                 VALUES ($1, $2, $3, $4, $5, $6, NULL) RETURNING id",
            )
            .bind(channel_id)
            .bind(dm_id)
            .bind(user_id)
            .bind(&body)
            .bind(reply_to)
            .bind(now_ms())
            .fetch_one(&mut *tx)
            .await
            .map_err(db_err)?;
            for (name, bytes) in files {
                let filename = safe_filename(name);
                let attachment_id: i64 = sqlx::query_scalar(
                    "INSERT INTO attachments (message_id, filename, byte_size) VALUES ($1, $2, $3) RETURNING id",
                )
                .bind(message_id)
                .bind(&filename)
                .bind(bytes.len() as i64)
                .fetch_one(&mut *tx)
                .await
                .map_err(db_err)?;
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
            replace_mentions_tx(&mut tx, message_id, user_id, &body, target).await?;
            let message = load_message_tx(&mut tx, message_id).await?;
            tx.commit().await.map_err(db_err)?;
            Ok(message)
        }
        .await;
        if result.is_err() {
            for path in written {
                let _ = fs::remove_file(path);
            }
        }
        result
    }

    pub async fn edit_message(&self, user_id: i64, message_id: i64, body: &str) -> Result<Message, BanterError> {
        let body = body.trim().to_string();
        if body.is_empty() {
            return Err(BanterError::EmptyMessage);
        }
        if body.chars().count() > MAX_MESSAGE_CHARS {
            return Err(BanterError::MessageTooLong);
        }
        let existing = load_message(&self.pool, message_id).await?;
        if existing.author_id != user_id {
            return Err(BanterError::NotAuthor);
        }
        let target = message_target_db(&self.pool, message_id).await?;
        sqlx::query("UPDATE messages SET body = $1, edited_at = $2 WHERE id = $3")
            .bind(&body)
            .bind(now_ms())
            .bind(message_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        replace_mentions(&self.pool, message_id, user_id, &body, target).await?;
        load_message(&self.pool, message_id).await
    }

    pub async fn delete_message(&self, user_id: i64, message_id: i64) -> Result<(), BanterError> {
        let files_dir = self.root.join("files");
        let existing = load_message(&self.pool, message_id).await?;
        if existing.author_id != user_id {
            return Err(BanterError::NotAuthor);
        }
        let paths: Vec<PathBuf> = existing
            .attachments
            .iter()
            .map(|attachment| files_dir.join(attachment.id.to_string()))
            .collect();
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        sqlx::query("UPDATE messages SET reply_to = NULL WHERE reply_to = $1")
            .bind(message_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        sqlx::query("DELETE FROM messages WHERE id = $1")
            .bind(message_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        tx.commit().await.map_err(db_err)?;
        for path in paths {
            let _ = fs::remove_file(path);
        }
        Ok(())
    }

    pub async fn messages(&self, user_id: i64, target: ChatTarget, limit: u32) -> Result<Vec<Message>, BanterError> {
        self.message_query(user_id, target, limit, None).await
    }

    pub async fn search_messages(&self, user_id: i64, target: ChatTarget, query: &str) -> Result<Vec<Message>, BanterError> {
        let query = query.trim();
        if query.is_empty() {
            return self.messages(user_id, target, 100).await;
        }
        self.message_query(user_id, target, 200, Some(query)).await
    }

    async fn message_query(
        &self,
        user_id: i64,
        target: ChatTarget,
        limit: u32,
        query: Option<&str>,
    ) -> Result<Vec<Message>, BanterError> {
        let limit = limit.clamp(1, 1000) as i64;
        require_text_access(&self.pool, user_id, target).await?;
        let (column, id) = match target {
            ChatTarget::Channel(id) => ("m.channel_id", id),
            ChatTarget::Direct(id) => ("m.dm_id", id),
        };
        let ids = if let Some(query) = query {
            let pattern = like_pattern(&query.to_lowercase());
            let sql = format!(
                "SELECT m.id FROM messages m
                 WHERE {column} = $1 AND lower(m.body) LIKE $2 ESCAPE '\\'
                 ORDER BY m.id DESC
                 LIMIT {limit}"
            );
            sqlx::query_scalar(&sql)
                .bind(id)
                .bind(pattern)
                .fetch_all(&self.pool)
                .await
                .map_err(db_err)?
        } else {
            let sql = format!(
                "SELECT m.id FROM messages m
                 WHERE {column} = $1
                 ORDER BY m.id DESC
                 LIMIT {limit}"
            );
            sqlx::query_scalar(&sql)
                .bind(id)
                .fetch_all(&self.pool)
                .await
                .map_err(db_err)?
        };
        let mut messages = Vec::with_capacity(ids.len());
        for id in ids.into_iter().rev() {
            messages.push(load_message(&self.pool, id).await?);
        }
        Ok(messages)
    }

    pub async fn read_attachment(&self, user_id: i64, attachment_id: i64) -> Result<(String, Vec<u8>), BanterError> {
        let row = sqlx::query("SELECT filename, message_id FROM attachments WHERE id = $1")
            .bind(attachment_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?
            .ok_or(BanterError::NotFound)?;
        let filename: String = row.get("filename");
        let message_id: i64 = row.get("message_id");
        let target = message_target_db(&self.pool, message_id).await?;
        require_text_access(&self.pool, user_id, target).await?;
        let bytes = fs::read(self.root.join("files").join(attachment_id.to_string()))
            .map_err(|err| BanterError::Storage(err.to_string()))?;
        Ok((filename, bytes))
    }

    pub async fn open_dm(&self, user_id: i64, peer_id: i64) -> Result<DirectChat, BanterError> {
        if user_id == peer_id {
            return Err(BanterError::NotFriend);
        }
        if !are_friends(&self.pool, user_id, peer_id).await? {
            return Err(BanterError::NotFriend);
        }
        let (low, high) = ordered_pair(user_id, peer_id);
        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM direct_messages WHERE user_a = $1 AND user_b = $2",
        )
        .bind(low)
        .bind(high)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        let id = if let Some(id) = existing {
            id
        } else {
            sqlx::query_scalar(
                "INSERT INTO direct_messages (user_a, user_b, created_at) VALUES ($1, $2, $3) RETURNING id",
            )
            .bind(low)
            .bind(high)
            .bind(now_ms())
            .fetch_one(&self.pool)
            .await
            .map_err(db_err)?
        };
        direct_chat(&self.pool, user_id, id).await
    }

    pub async fn direct_messages(&self, user_id: i64) -> Result<Vec<DirectChat>, BanterError> {
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM direct_messages WHERE user_a = $1 OR user_b = $1 ORDER BY id",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        let mut chats = Vec::new();
        for id in ids {
            chats.push(direct_chat(&self.pool, user_id, id).await?);
        }
        Ok(chats)
    }

    pub async fn unread_mentions(&self, user_id: i64) -> Result<Vec<Mention>, BanterError> {
        let rows = sqlx::query(
            "SELECT mn.id, mn.message_id, m.channel_id, m.dm_id, u.username AS from_username, m.body
             FROM mentions mn
             JOIN messages m ON m.id = mn.message_id
             JOIN users u ON u.id = m.author_id
             WHERE mn.user_id = $1 AND mn.read_at IS NULL
             ORDER BY mn.id",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows
            .iter()
            .map(|row| Mention {
                id: row.get("id"),
                message_id: row.get("message_id"),
                channel_id: row.get("channel_id"),
                dm_id: row.get("dm_id"),
                from_username: row.get("from_username"),
                body: row.get("body"),
            })
            .collect())
    }

    pub async fn mark_mention_read(&self, user_id: i64, mention_id: i64) -> Result<(), BanterError> {
        let result = sqlx::query("UPDATE mentions SET read_at = $1 WHERE id = $2 AND user_id = $3")
            .bind(now_ms())
            .bind(mention_id)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        if result.rows_affected() == 0 {
            return Err(BanterError::NotFound);
        }
        Ok(())
    }

    pub(crate) async fn channel_access(&self, user_id: i64, channel_id: i64) -> Result<Channel, BanterError> {
        let channel = channel_by_id(&self.pool, channel_id).await?;
        if !is_member(&self.pool, user_id, channel.server_id).await? {
            return Err(BanterError::NotMember);
        }
        Ok(channel)
    }

    pub(crate) async fn dm_participants(&self, user_id: i64, dm_id: i64) -> Result<(i64, i64), BanterError> {
        let row = sqlx::query("SELECT user_a, user_b FROM direct_messages WHERE id = $1")
            .bind(dm_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        let Some(row) = row else {
            return Err(BanterError::NotFound);
        };
        let low: i64 = row.get("user_a");
        let high: i64 = row.get("user_b");
        if user_id == low {
            Ok((low, high))
        } else if user_id == high {
            Ok((high, low))
        } else {
            Err(BanterError::NotMember)
        }
    }

    pub(crate) async fn user_by_username(&self, username: &str) -> Result<Account, BanterError> {
        let username = normalize_username(username)?;
        account_by_username(&self.pool, &username).await
    }

    pub(crate) async fn target_user_ids(&self, target: ChatTarget) -> Result<Vec<i64>, BanterError> {
        visible_user_ids(&self.pool, target).await
    }

    pub(crate) async fn message_target(&self, message_id: i64) -> Result<ChatTarget, BanterError> {
        message_target_db(&self.pool, message_id).await
    }

    pub(crate) async fn related_user_ids(&self, user_id: i64) -> Result<Vec<i64>, BanterError> {
        let mut ids: Vec<i64> = sqlx::query_scalar(
            "SELECT user_b FROM friendships WHERE user_a = $1
             UNION SELECT user_a FROM friendships WHERE user_b = $1
             UNION SELECT m2.user_id FROM memberships m1
             JOIN memberships m2 ON m1.server_id = m2.server_id
             WHERE m1.user_id = $1",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        if !ids.contains(&user_id) {
            ids.push(user_id);
        }
        Ok(ids)
    }

    async fn server_home(&self, user_id: i64, server_id: i64) -> Result<ServerHome, BanterError> {
        let _ = user_id;
        let row = sqlx::query("SELECT id, name, owner_id FROM servers WHERE id = $1")
            .bind(server_id)
            .fetch_one(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(ServerHome {
            server: ServerSummary {
                id: row.get("id"),
                name: row.get("name"),
                owner_id: row.get("owner_id"),
            },
            categories: layout_of(&self.pool, server_id).await?,
        })
    }

    async fn require_member(&self, user_id: i64, server_id: i64) -> Result<(), BanterError> {
        if is_member(&self.pool, user_id, server_id).await? {
            Ok(())
        } else {
            Err(BanterError::NotMember)
        }
    }

    async fn require_owner(&self, user_id: i64, server_id: i64) -> Result<(), BanterError> {
        if !is_member(&self.pool, user_id, server_id).await? {
            return Err(BanterError::NotMember);
        }
        let owner: i64 = sqlx::query_scalar("SELECT owner_id FROM servers WHERE id = $1")
            .bind(server_id)
            .fetch_one(&self.pool)
            .await
            .map_err(db_err)?;
        if owner == user_id {
            Ok(())
        } else {
            Err(BanterError::NotOwner)
        }
    }
}

async fn account_by_id(pool: &PgPool, id: i64) -> Result<Account, BanterError> {
    let row = sqlx::query(
        "SELECT id, username, display_name, avatar_file, avatar_rev FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(db_err)?
    .ok_or(BanterError::NotFound)?;
    Ok(map_account_row(&row))
}

async fn account_by_username(pool: &PgPool, username: &str) -> Result<Account, BanterError> {
    let row = sqlx::query(
        "SELECT id, username, display_name, avatar_file, avatar_rev FROM users WHERE username = $1",
    )
    .bind(username)
    .fetch_optional(pool)
    .await
    .map_err(db_err)?
    .ok_or(BanterError::NotFound)?;
    Ok(map_account_row(&row))
}

fn map_account_row(row: &sqlx::postgres::PgRow) -> Account {
    Account {
        id: row.get("id"),
        username: row.get("username"),
        display_name: row.get("display_name"),
        has_avatar: row.get::<Option<String>, _>("avatar_file").is_some(),
        avatar_rev: row.get("avatar_rev"),
    }
}

fn map_request_row(row: &sqlx::postgres::PgRow) -> FriendRequest {
    FriendRequest {
        id: row.get("id"),
        from_id: row.get("from_user"),
        from_username: row.get("from_username"),
        to_id: row.get("to_user"),
        to_username: row.get("to_username"),
    }
}

async fn request_by_id(pool: &PgPool, id: i64) -> Result<FriendRequest, BanterError> {
    let row = sqlx::query(
        "SELECT r.id, r.from_user, fu.username AS from_username, r.to_user, tu.username AS to_username
         FROM friend_requests r
         JOIN users fu ON fu.id = r.from_user
         JOIN users tu ON tu.id = r.to_user
         WHERE r.id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(db_err)?
    .ok_or(BanterError::NotPending)?;
    Ok(map_request_row(&row))
}

async fn pending_between(pool: &PgPool, a: i64, b: i64) -> Result<Option<FriendRequest>, BanterError> {
    let row = sqlx::query(
        "SELECT r.id, r.from_user, fu.username AS from_username, r.to_user, tu.username AS to_username
         FROM friend_requests r
         JOIN users fu ON fu.id = r.from_user
         JOIN users tu ON tu.id = r.to_user
         WHERE (r.from_user = $1 AND r.to_user = $2) OR (r.from_user = $2 AND r.to_user = $1)",
    )
    .bind(a)
    .bind(b)
    .fetch_optional(pool)
    .await
    .map_err(db_err)?;
    Ok(row.as_ref().map(map_request_row))
}

async fn accept_request(pool: &PgPool, user_id: i64, request_id: i64) -> Result<(), BanterError> {
    let request = request_by_id(pool, request_id).await?;
    if request.to_id != user_id {
        return Err(BanterError::NotPending);
    }
    let (low, high) = ordered_pair(request.from_id, request.to_id);
    sqlx::query("INSERT INTO friendships (user_a, user_b, created_at) VALUES ($1, $2, $3)")
        .bind(low)
        .bind(high)
        .bind(now_ms())
        .execute(pool)
        .await
        .map_err(db_err)?;
    sqlx::query("DELETE FROM friend_requests WHERE id = $1")
        .bind(request_id)
        .execute(pool)
        .await
        .map_err(db_err)?;
    Ok(())
}

async fn are_friends(pool: &PgPool, a: i64, b: i64) -> Result<bool, BanterError> {
    let (low, high) = ordered_pair(a, b);
    let found: Option<bool> = sqlx::query_scalar("SELECT TRUE FROM friendships WHERE user_a = $1 AND user_b = $2")
        .bind(low)
        .bind(high)
        .fetch_optional(pool)
        .await
        .map_err(db_err)?;
    Ok(found.is_some())
}

fn ordered_pair(a: i64, b: i64) -> (i64, i64) {
    if a < b { (a, b) } else { (b, a) }
}

async fn is_member(pool: &PgPool, user_id: i64, server_id: i64) -> Result<bool, BanterError> {
    let found: Option<bool> =
        sqlx::query_scalar("SELECT TRUE FROM memberships WHERE user_id = $1 AND server_id = $2")
            .bind(user_id)
            .bind(server_id)
            .fetch_optional(pool)
            .await
            .map_err(db_err)?;
    Ok(found.is_some())
}

async fn members_of(pool: &PgPool, server_id: i64) -> Result<Vec<Member>, BanterError> {
    let owner: i64 = sqlx::query_scalar("SELECT owner_id FROM servers WHERE id = $1")
        .bind(server_id)
        .fetch_one(pool)
        .await
        .map_err(db_err)?;
    let rows = sqlx::query(
        "SELECT u.id, u.username, u.display_name, m.nickname, u.avatar_file, u.avatar_rev
         FROM memberships m
         JOIN users u ON u.id = m.user_id
         WHERE m.server_id = $1
         ORDER BY u.username",
    )
    .bind(server_id)
    .fetch_all(pool)
    .await
    .map_err(db_err)?;
    Ok(rows
        .iter()
        .map(|row| {
            let user_id: i64 = row.get("id");
            Member {
                user_id,
                username: row.get("username"),
                display_name: row.get("display_name"),
                nickname: row.get("nickname"),
                is_owner: user_id == owner,
                has_avatar: row.get::<Option<String>, _>("avatar_file").is_some(),
                avatar_rev: row.get("avatar_rev"),
            }
        })
        .collect())
}

async fn insert_category_tx(
    tx: &mut Transaction<'_, Postgres>,
    server_id: i64,
    name: &str,
    position: i64,
) -> Result<i64, BanterError> {
    sqlx::query_scalar("INSERT INTO categories (server_id, name, position) VALUES ($1, $2, $3) RETURNING id")
        .bind(server_id)
        .bind(name)
        .bind(position)
        .fetch_one(&mut **tx)
        .await
        .map_err(db_err)
}

async fn insert_channel_tx(
    tx: &mut Transaction<'_, Postgres>,
    server_id: i64,
    category_id: i64,
    name: &str,
    kind: ChannelKind,
    position: i64,
) -> Result<i64, BanterError> {
    sqlx::query_scalar(
        "INSERT INTO channels (server_id, category_id, name, kind, position)
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(server_id)
    .bind(category_id)
    .bind(name)
    .bind(kind.as_str())
    .bind(position)
    .fetch_one(&mut **tx)
    .await
    .map_err(db_err)
}

async fn insert_empty_board_scene(pool: &PgPool, channel_id: i64) -> Result<(), BanterError> {
    let json = serde_json::to_string(&BoardScene::empty()).map_err(|err| BanterError::Storage(err.to_string()))?;
    sqlx::query("INSERT INTO board_scenes (channel_id, revision, \"json\") VALUES ($1, 0, $2)")
        .bind(channel_id)
        .bind(json)
        .execute(pool)
        .await
        .map_err(db_err)?;
    Ok(())
}

async fn load_board_scene(pool: &PgPool, channel_id: i64) -> Result<BoardScene, BanterError> {
    let row = sqlx::query("SELECT revision, \"json\" FROM board_scenes WHERE channel_id = $1")
        .bind(channel_id)
        .fetch_optional(pool)
        .await
        .map_err(db_err)?
        .ok_or(BanterError::NotFound)?;
    let revision: i64 = row.get("revision");
    let json: String = row.get("json");
    let mut scene: BoardScene = serde_json::from_str(&json).map_err(|err| BanterError::Storage(err.to_string()))?;
    scene.revision = revision;
    Ok(scene)
}

async fn save_board_scene(pool: &PgPool, channel_id: i64, scene: &BoardScene) -> Result<(), BanterError> {
    let json = serde_json::to_string(scene).map_err(|err| BanterError::Storage(err.to_string()))?;
    sqlx::query("UPDATE board_scenes SET revision = $1, \"json\" = $2 WHERE channel_id = $3")
        .bind(scene.revision)
        .bind(json)
        .bind(channel_id)
        .execute(pool)
        .await
        .map_err(db_err)?;
    Ok(())
}

async fn require_board_access(pool: &PgPool, user_id: i64, channel_id: i64) -> Result<(), BanterError> {
    let channel = channel_by_id(pool, channel_id).await?;
    if !is_member(pool, user_id, channel.server_id).await? {
        return Err(BanterError::NotMember);
    }
    if channel.kind != ChannelKind::Board {
        return Err(BanterError::NotBoardChannel);
    }
    Ok(())
}

fn apply_op_to_scene(scene: &mut BoardScene, op: BoardOp, user_id: i64) -> Result<(), BanterError> {
    match op {
        BoardOp::Upsert { elements, .. } => {
            for mut element in elements {
                element.updated_by = user_id;
                if let Some(existing) = scene.elements.iter_mut().find(|item| item.id == element.id) {
                    *existing = element;
                } else {
                    scene.elements.push(element);
                }
            }
        }
        BoardOp::Delete { ids, .. } => {
            scene.elements.retain(|element| !ids.contains(&element.id));
        }
        BoardOp::Reorder { ids, .. } => {
            let mut ordered = Vec::new();
            for (z, id) in ids.iter().enumerate() {
                if let Some(mut element) = scene.elements.iter().find(|item| &item.id == id).cloned() {
                    element.z = z as i64;
                    element.updated_by = user_id;
                    ordered.push(element);
                }
            }
            for element in &scene.elements {
                if !ids.contains(&element.id) {
                    ordered.push(element.clone());
                }
            }
            scene.elements = ordered;
        }
        BoardOp::Group { ids, group_id, .. } => {
            for element in &mut scene.elements {
                if ids.contains(&element.id) {
                    element.group_id = Some(group_id.clone());
                    element.updated_by = user_id;
                }
            }
        }
        BoardOp::Ungroup { group_ids, .. } => {
            for element in &mut scene.elements {
                if element.group_id.as_ref().is_some_and(|id| group_ids.contains(id)) {
                    element.group_id = None;
                    element.updated_by = user_id;
                }
            }
        }
    }
    scene.elements.sort_by_key(|element| element.z);
    Ok(())
}

async fn layout_of(pool: &PgPool, server_id: i64) -> Result<Vec<CategoryLayout>, BanterError> {
    let categories = sqlx::query(
        "SELECT id, server_id, name, position FROM categories WHERE server_id = $1 ORDER BY position, id",
    )
    .bind(server_id)
    .fetch_all(pool)
    .await
    .map_err(db_err)?;
    let mut layouts = Vec::new();
    for row in categories {
        let category = Category {
            id: row.get("id"),
            server_id: row.get("server_id"),
            name: row.get("name"),
            position: row.get("position"),
        };
        let channel_rows = sqlx::query(
            "SELECT id, server_id, category_id, name, kind, position
             FROM channels WHERE category_id = $1 ORDER BY position, id",
        )
        .bind(category.id)
        .fetch_all(pool)
        .await
        .map_err(db_err)?;
        let channels = channel_rows
            .iter()
            .map(|row| Channel {
                id: row.get("id"),
                server_id: row.get("server_id"),
                category_id: row.get("category_id"),
                name: row.get("name"),
                kind: ChannelKind::parse(&row.get::<String, _>("kind")).unwrap_or(ChannelKind::Text),
                position: row.get("position"),
            })
            .collect();
        layouts.push(CategoryLayout { category, channels });
    }
    Ok(layouts)
}

async fn channel_by_id(pool: &PgPool, id: i64) -> Result<Channel, BanterError> {
    let row = sqlx::query(
        "SELECT id, server_id, category_id, name, kind, position FROM channels WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(db_err)?
    .ok_or(BanterError::NotFound)?;
    Ok(Channel {
        id: row.get("id"),
        server_id: row.get("server_id"),
        category_id: row.get("category_id"),
        name: row.get("name"),
        kind: ChannelKind::parse(&row.get::<String, _>("kind")).unwrap_or(ChannelKind::Text),
        position: row.get("position"),
    })
}

async fn require_text_access(pool: &PgPool, user_id: i64, target: ChatTarget) -> Result<(), BanterError> {
    match target {
        ChatTarget::Channel(channel_id) => {
            let channel = channel_by_id(pool, channel_id).await?;
            if !is_member(pool, user_id, channel.server_id).await? {
                return Err(BanterError::NotMember);
            }
            if channel.kind != ChannelKind::Text {
                return Err(BanterError::NotTextChannel);
            }
            Ok(())
        }
        ChatTarget::Direct(dm_id) => {
            let row = sqlx::query("SELECT user_a, user_b FROM direct_messages WHERE id = $1")
                .bind(dm_id)
                .fetch_optional(pool)
                .await
                .map_err(db_err)?;
            let Some(row) = row else {
                return Err(BanterError::NotFound);
            };
            let low: i64 = row.get("user_a");
            let high: i64 = row.get("user_b");
            if user_id == low || user_id == high {
                Ok(())
            } else {
                Err(BanterError::NotMember)
            }
        }
    }
}

async fn require_text_access_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
    target: ChatTarget,
) -> Result<(), BanterError> {
    match target {
        ChatTarget::Channel(channel_id) => {
            let row = sqlx::query(
                "SELECT id, server_id, category_id, name, kind, position FROM channels WHERE id = $1",
            )
            .bind(channel_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db_err)?
            .ok_or(BanterError::NotFound)?;
            let channel = Channel {
                id: row.get("id"),
                server_id: row.get("server_id"),
                category_id: row.get("category_id"),
                name: row.get("name"),
                kind: ChannelKind::parse(&row.get::<String, _>("kind")).unwrap_or(ChannelKind::Text),
                position: row.get("position"),
            };
            let found: Option<bool> =
                sqlx::query_scalar("SELECT TRUE FROM memberships WHERE user_id = $1 AND server_id = $2")
                    .bind(user_id)
                    .bind(channel.server_id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(db_err)?;
            if found.is_none() {
                return Err(BanterError::NotMember);
            }
            if channel.kind != ChannelKind::Text {
                return Err(BanterError::NotTextChannel);
            }
            Ok(())
        }
        ChatTarget::Direct(dm_id) => {
            let row = sqlx::query("SELECT user_a, user_b FROM direct_messages WHERE id = $1")
                .bind(dm_id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(db_err)?;
            let Some(row) = row else {
                return Err(BanterError::NotFound);
            };
            let low: i64 = row.get("user_a");
            let high: i64 = row.get("user_b");
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

async fn reply_in_target_tx(
    tx: &mut Transaction<'_, Postgres>,
    reply_id: i64,
    target: ChatTarget,
) -> Result<bool, BanterError> {
    match message_target_tx(tx, reply_id).await {
        Ok(existing) => Ok(existing == target),
        Err(BanterError::NotFound) => Ok(false),
        Err(err) => Err(err),
    }
}

async fn message_target_db(pool: &PgPool, message_id: i64) -> Result<ChatTarget, BanterError> {
    let row = sqlx::query("SELECT channel_id, dm_id FROM messages WHERE id = $1")
        .bind(message_id)
        .fetch_optional(pool)
        .await
        .map_err(db_err)?;
    match row {
        Some(row) => {
            let channel_id: Option<i64> = row.get("channel_id");
            let dm_id: Option<i64> = row.get("dm_id");
            match (channel_id, dm_id) {
                (Some(channel), _) => Ok(ChatTarget::Channel(channel)),
                (_, Some(dm)) => Ok(ChatTarget::Direct(dm)),
                _ => Err(BanterError::NotFound),
            }
        }
        None => Err(BanterError::NotFound),
    }
}

async fn message_target_tx(
    tx: &mut Transaction<'_, Postgres>,
    message_id: i64,
) -> Result<ChatTarget, BanterError> {
    let row = sqlx::query("SELECT channel_id, dm_id FROM messages WHERE id = $1")
        .bind(message_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(db_err)?;
    match row {
        Some(row) => {
            let channel_id: Option<i64> = row.get("channel_id");
            let dm_id: Option<i64> = row.get("dm_id");
            match (channel_id, dm_id) {
                (Some(channel), _) => Ok(ChatTarget::Channel(channel)),
                (_, Some(dm)) => Ok(ChatTarget::Direct(dm)),
                _ => Err(BanterError::NotFound),
            }
        }
        None => Err(BanterError::NotFound),
    }
}

async fn load_message(pool: &PgPool, message_id: i64) -> Result<Message, BanterError> {
    let row = sqlx::query(
        "SELECT m.id, m.author_id, u.username, u.display_name, mem.nickname,
                m.body, m.reply_to, m.created_at, m.edited_at
         FROM messages m
         JOIN users u ON u.id = m.author_id
         LEFT JOIN channels c ON c.id = m.channel_id
         LEFT JOIN memberships mem ON mem.user_id = m.author_id AND mem.server_id = c.server_id
         WHERE m.id = $1",
    )
    .bind(message_id)
    .fetch_optional(pool)
    .await
    .map_err(db_err)?
    .ok_or(BanterError::NotFound)?;
    let attachments_rows = sqlx::query(
        "SELECT id, filename, byte_size FROM attachments WHERE message_id = $1 ORDER BY id",
    )
    .bind(message_id)
    .fetch_all(pool)
    .await
    .map_err(db_err)?;
    let attachments = attachments_rows
        .iter()
        .map(|row| Attachment {
            id: row.get("id"),
            filename: row.get("filename"),
            byte_size: row.get("byte_size"),
        })
        .collect();
    Ok(Message {
        id: row.get("id"),
        author_id: row.get("author_id"),
        author_username: row.get("username"),
        author_display_name: row.get("display_name"),
        author_nickname: row.get("nickname"),
        body: row.get("body"),
        reply_to: row.get("reply_to"),
        created_at: row.get("created_at"),
        edited_at: row.get("edited_at"),
        attachments,
    })
}

async fn load_message_tx(tx: &mut Transaction<'_, Postgres>, message_id: i64) -> Result<Message, BanterError> {
    let row = sqlx::query(
        "SELECT m.id, m.author_id, u.username, u.display_name, mem.nickname,
                m.body, m.reply_to, m.created_at, m.edited_at
         FROM messages m
         JOIN users u ON u.id = m.author_id
         LEFT JOIN channels c ON c.id = m.channel_id
         LEFT JOIN memberships mem ON mem.user_id = m.author_id AND mem.server_id = c.server_id
         WHERE m.id = $1",
    )
    .bind(message_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(db_err)?
    .ok_or(BanterError::NotFound)?;
    let attachments_rows = sqlx::query(
        "SELECT id, filename, byte_size FROM attachments WHERE message_id = $1 ORDER BY id",
    )
    .bind(message_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(db_err)?;
    let attachments = attachments_rows
        .iter()
        .map(|row| Attachment {
            id: row.get("id"),
            filename: row.get("filename"),
            byte_size: row.get("byte_size"),
        })
        .collect();
    Ok(Message {
        id: row.get("id"),
        author_id: row.get("author_id"),
        author_username: row.get("username"),
        author_display_name: row.get("display_name"),
        author_nickname: row.get("nickname"),
        body: row.get("body"),
        reply_to: row.get("reply_to"),
        created_at: row.get("created_at"),
        edited_at: row.get("edited_at"),
        attachments,
    })
}

async fn replace_mentions(
    pool: &PgPool,
    message_id: i64,
    author_id: i64,
    body: &str,
    target: ChatTarget,
) -> Result<(), BanterError> {
    sqlx::query("DELETE FROM mentions WHERE message_id = $1")
        .bind(message_id)
        .execute(pool)
        .await
        .map_err(db_err)?;
    let visible = visible_users(pool, target).await?;
    for token in mention_usernames(body) {
        let Some((user_id, _)) = visible.iter().find(|(_, username)| username == &token) else {
            continue;
        };
        if *user_id == author_id {
            continue;
        }
        sqlx::query("INSERT INTO mentions (message_id, user_id, read_at) VALUES ($1, $2, NULL)")
            .bind(message_id)
            .bind(user_id)
            .execute(pool)
            .await
            .map_err(db_err)?;
    }
    Ok(())
}

async fn replace_mentions_tx(
    tx: &mut Transaction<'_, Postgres>,
    message_id: i64,
    author_id: i64,
    body: &str,
    target: ChatTarget,
) -> Result<(), BanterError> {
    sqlx::query("DELETE FROM mentions WHERE message_id = $1")
        .bind(message_id)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
    let visible = visible_users_tx(tx, target).await?;
    for token in mention_usernames(body) {
        let Some((user_id, _)) = visible.iter().find(|(_, username)| username == &token) else {
            continue;
        };
        if *user_id == author_id {
            continue;
        }
        sqlx::query("INSERT INTO mentions (message_id, user_id, read_at) VALUES ($1, $2, NULL)")
            .bind(message_id)
            .bind(user_id)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
    }
    Ok(())
}

async fn visible_users(pool: &PgPool, target: ChatTarget) -> Result<Vec<(i64, String)>, BanterError> {
    match target {
        ChatTarget::Channel(channel_id) => {
            let server_id: i64 = sqlx::query_scalar("SELECT server_id FROM channels WHERE id = $1")
                .bind(channel_id)
                .fetch_one(pool)
                .await
                .map_err(db_err)?;
            let rows = sqlx::query(
                "SELECT u.id, u.username FROM memberships m
                 JOIN users u ON u.id = m.user_id
                 WHERE m.server_id = $1",
            )
            .bind(server_id)
            .fetch_all(pool)
            .await
            .map_err(db_err)?;
            Ok(rows.iter().map(|row| (row.get("id"), row.get("username"))).collect())
        }
        ChatTarget::Direct(dm_id) => {
            let row = sqlx::query("SELECT user_a, user_b FROM direct_messages WHERE id = $1")
                .bind(dm_id)
                .fetch_one(pool)
                .await
                .map_err(db_err)?;
            let low: i64 = row.get("user_a");
            let high: i64 = row.get("user_b");
            let mut users = Vec::new();
            for id in [low, high] {
                let account = account_by_id(pool, id).await?;
                users.push((account.id, account.username));
            }
            Ok(users)
        }
    }
}

async fn visible_users_tx(
    tx: &mut Transaction<'_, Postgres>,
    target: ChatTarget,
) -> Result<Vec<(i64, String)>, BanterError> {
    match target {
        ChatTarget::Channel(channel_id) => {
            let server_id: i64 = sqlx::query_scalar("SELECT server_id FROM channels WHERE id = $1")
                .bind(channel_id)
                .fetch_one(&mut **tx)
                .await
                .map_err(db_err)?;
            let rows = sqlx::query(
                "SELECT u.id, u.username FROM memberships m
                 JOIN users u ON u.id = m.user_id
                 WHERE m.server_id = $1",
            )
            .bind(server_id)
            .fetch_all(&mut **tx)
            .await
            .map_err(db_err)?;
            Ok(rows.iter().map(|row| (row.get("id"), row.get("username"))).collect())
        }
        ChatTarget::Direct(dm_id) => {
            let row = sqlx::query("SELECT user_a, user_b FROM direct_messages WHERE id = $1")
                .bind(dm_id)
                .fetch_one(&mut **tx)
                .await
                .map_err(db_err)?;
            let low: i64 = row.get("user_a");
            let high: i64 = row.get("user_b");
            let mut users = Vec::new();
            for id in [low, high] {
                let account_row = sqlx::query(
                    "SELECT id, username, display_name, avatar_file, avatar_rev FROM users WHERE id = $1",
                )
                .bind(id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(db_err)?
                .ok_or(BanterError::NotFound)?;
                users.push((account_row.get("id"), account_row.get("username")));
            }
            Ok(users)
        }
    }
}

async fn visible_user_ids(pool: &PgPool, target: ChatTarget) -> Result<Vec<i64>, BanterError> {
    Ok(visible_users(pool, target).await?.into_iter().map(|(id, _)| id).collect())
}

async fn direct_chat(pool: &PgPool, viewer: i64, dm_id: i64) -> Result<DirectChat, BanterError> {
    let row = sqlx::query("SELECT user_a, user_b FROM direct_messages WHERE id = $1")
        .bind(dm_id)
        .fetch_one(pool)
        .await
        .map_err(db_err)?;
    let low: i64 = row.get("user_a");
    let high: i64 = row.get("user_b");
    let peer_id = if viewer == low { high } else { low };
    let peer = account_by_id(pool, peer_id).await?;
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

fn db_err(err: sqlx::Error) -> BanterError {
    BanterError::Storage(err.to_string())
}

fn constraint(err: sqlx::Error, unique: BanterError) -> BanterError {
    if is_unique_violation(&err) {
        unique
    } else {
        db_err(err)
    }
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    match err {
        sqlx::Error::Database(db) => db.code().as_deref() == Some("23505"),
        _ => false,
    }
}
