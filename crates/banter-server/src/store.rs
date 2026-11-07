use std::collections::HashMap;
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
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::badges::{badge_exists, badge_item, MAX_EQUIPPED_BADGES};
use crate::cosmetics::{cosmetic_exists, CosmeticKind, DEFAULT_THEME_PRIMARY, DEFAULT_THEME_SECONDARY};
use crate::error::BanterError;
use crate::folders::{
    child_depth, folder_color_ok, would_cycle, MAX_SERVER_FOLDERS, MAX_SERVER_FOLDER_DEPTH,
};
use crate::model::*;
use crate::roles::{
    grants_only_held, has_permission, next_role_position, role_color_ok, EVERYONE_NAME, EVERYONE_PERMISSIONS,
    MAX_GUILD_ROLES, PERM_ADMINISTRATOR, PERM_CREATE_INVITE, PERM_MANAGE_CHANNELS, PERM_MANAGE_ROLES,
    ROLE_PERMISSION_MASK,
};

pub struct Store {
    pub(crate) pool: PgPool,
    pub(crate) root: PathBuf,
    pub(crate) plugins: Mutex<HashMap<(i64, String), crate::script::Addon>>,
    pub(crate) rates: Mutex<crate::bots::Rates>,
}

impl Store {
    pub async fn open(root: &Path, database_url: &str) -> Result<Self, BanterError> {
        fs::create_dir_all(root).map_err(|err| BanterError::Storage(err.to_string()))?;
        fs::create_dir_all(root.join("files")).map_err(|err| BanterError::Storage(err.to_string()))?;
        fs::create_dir_all(root.join("files").join("board")).map_err(|err| BanterError::Storage(err.to_string()))?;
        fs::create_dir_all(root.join("avatars")).map_err(|err| BanterError::Storage(err.to_string()))?;
        fs::create_dir_all(root.join("banners")).map_err(|err| BanterError::Storage(err.to_string()))?;
        fs::create_dir_all(root.join("emotes")).map_err(|err| BanterError::Storage(err.to_string()))?;
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await
            .map_err(db_err)?;
        // Greenfield: CREATE IF NOT EXISTS for the current tip schema.
        sqlx::raw_sql(include_str!("schema.sql"))
            .execute(&pool)
            .await
            .map_err(db_err)?;
        // Existing volumes: versioned ALTER / new-table upgrades (see migrate.rs).
        crate::migrate::apply(&pool).await?;
        Ok(Self {
            pool,
            root: root.to_path_buf(),
            plugins: Mutex::new(HashMap::new()),
            rates: Mutex::new(crate::bots::Rates::default()),
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
        let hash: Option<String> = row.get("password_hash");
        let Some(hash) = hash else {
            return Err(BanterError::InvalidCredentials);
        };
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
        let bot: Option<i64> = sqlx::query_scalar("SELECT user_id FROM bot_tokens WHERE token = $1")
            .bind(token)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        if bot.is_some() {
            return Err(BanterError::Invalid);
        }
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
        if let Some(id) = id {
            return account_by_id(&self.pool, id).await;
        }
        let id: Option<i64> = sqlx::query_scalar("SELECT user_id FROM bot_tokens WHERE token = $1")
            .bind(token)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        let Some(id) = id else {
            return Err(BanterError::InvalidCredentials);
        };
        account_by_id(&self.pool, id).await
    }

    pub async fn custom_status(&self, user_id: i64) -> Result<Option<String>, BanterError> {
        let status: Option<String> = sqlx::query_scalar("SELECT custom_status FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?
            .flatten();
        Ok(status)
    }

    pub async fn account(&self, user_id: i64) -> Result<Account, BanterError> {
        account_by_id(&self.pool, user_id).await
    }

    /// Profile card for `target_id` as seen by `viewer_id`.
    ///
    /// Self, friends, and shared-server members can view. Strangers get
    /// `NotFound`. When `server_id` is set, the viewer must be a member of that
    /// server (`NotMember` otherwise), and the target's nickname there is
    /// included when they are also a member.
    pub async fn user_profile(
        &self,
        viewer_id: i64,
        target_id: i64,
        server_id: Option<i64>,
    ) -> Result<UserProfile, BanterError> {
        if let Some(server_id) = server_id {
            if !is_member(&self.pool, viewer_id, server_id).await? {
                return Err(BanterError::NotMember);
            }
        }
        let is_self = viewer_id == target_id;
        let is_friend = !is_self && are_friends(&self.pool, viewer_id, target_id).await?;
        let can_view = is_self || is_friend || share_server(&self.pool, viewer_id, target_id).await?;
        if !can_view {
            return Err(BanterError::NotFound);
        }
        let nickname = match server_id {
            Some(server_id) => member_nickname(&self.pool, server_id, target_id).await?,
            None => None,
        };
        let mut profile = load_user_profile(&self.pool, target_id, nickname, is_self, is_friend).await?;
        if let Some(server_id) = server_id {
            if is_member(&self.pool, target_id, server_id).await? {
                profile.roles = roles_for_member(&self.pool, server_id, target_id).await?;
            }
        }
        Ok(profile)
    }

    /// Apply a Discord-parity profile customization patch for the owner.
    pub async fn update_profile(&self, user_id: i64, patch: ProfilePatch) -> Result<UserProfile, BanterError> {
        let _ = account_by_id(&self.pool, user_id).await?;
        if let Some(ref name) = patch.display_name {
            let display_name = clean_display(name)?;
            sqlx::query("UPDATE users SET display_name = $1 WHERE id = $2")
                .bind(&display_name)
                .bind(user_id)
                .execute(&self.pool)
                .await
                .map_err(db_err)?;
        }
        if let Some(ref about) = patch.about_me {
            let about_me = clean_optional_text(about, MAX_ABOUT_ME_CHARS)?;
            sqlx::query("UPDATE users SET about_me = $1 WHERE id = $2")
                .bind(about_me)
                .bind(user_id)
                .execute(&self.pool)
                .await
                .map_err(db_err)?;
        }
        if let Some(ref pronouns) = patch.pronouns {
            let pronouns = clean_optional_text(pronouns, MAX_PRONOUNS_CHARS)?;
            sqlx::query("UPDATE users SET pronouns = $1 WHERE id = $2")
                .bind(pronouns)
                .bind(user_id)
                .execute(&self.pool)
                .await
                .map_err(db_err)?;
        }
        if let Some(ref status) = patch.custom_status {
            let custom_status = clean_optional_text(status, MAX_CUSTOM_STATUS_CHARS)?;
            sqlx::query("UPDATE users SET custom_status = $1 WHERE id = $2")
                .bind(custom_status)
                .bind(user_id)
                .execute(&self.pool)
                .await
                .map_err(db_err)?;
        }
        if let Some(primary) = patch.theme_primary {
            sqlx::query("UPDATE users SET theme_primary = $1 WHERE id = $2")
                .bind(primary as i64)
                .bind(user_id)
                .execute(&self.pool)
                .await
                .map_err(db_err)?;
        }
        if let Some(secondary) = patch.theme_secondary {
            sqlx::query("UPDATE users SET theme_secondary = $1 WHERE id = $2")
                .bind(secondary as i64)
                .bind(user_id)
                .execute(&self.pool)
                .await
                .map_err(db_err)?;
        }
        apply_cosmetic(
            &self.pool,
            user_id,
            "nameplate_id",
            patch.nameplate_id.as_deref(),
            CosmeticKind::Nameplate,
        )
        .await?;
        apply_cosmetic(
            &self.pool,
            user_id,
            "avatar_decoration_id",
            patch.avatar_decoration_id.as_deref(),
            CosmeticKind::AvatarDecoration,
        )
        .await?;
        apply_cosmetic(
            &self.pool,
            user_id,
            "display_name_style_id",
            patch.display_name_style_id.as_deref(),
            CosmeticKind::DisplayNameStyle,
        )
        .await?;
        apply_cosmetic(
            &self.pool,
            user_id,
            "profile_effect_id",
            patch.profile_effect_id.as_deref(),
            CosmeticKind::ProfileEffect,
        )
        .await?;
        apply_cosmetic(
            &self.pool,
            user_id,
            "profile_frame_id",
            patch.profile_frame_id.as_deref(),
            CosmeticKind::ProfileFrame,
        )
        .await?;
        if let Some(widgets) = patch.widgets {
            replace_widgets(&self.pool, user_id, &widgets).await?;
        }
        if let Some(connections) = patch.connections {
            replace_connections(&self.pool, user_id, &connections).await?;
        }
        if let Some(badge_ids) = patch.badge_ids {
            replace_badges(&self.pool, user_id, &badge_ids).await?;
        }
        if let Some(activity) = patch.activity {
            replace_activity(&self.pool, user_id, &activity).await?;
        }
        if let Some(wishlist) = patch.wishlist {
            replace_wishlist(&self.pool, user_id, &wishlist).await?;
        }
        load_user_profile(&self.pool, user_id, None, true, false).await
    }

    pub async fn set_banner(&self, user_id: i64, bytes: &[u8]) -> Result<(), BanterError> {
        if bytes.is_empty() || bytes.len() > MAX_BANNER_BYTES {
            return Err(BanterError::InvalidBanner);
        }
        let decoded = image::load_from_memory(bytes).map_err(|_| BanterError::InvalidBanner)?;
        let scaled = if decoded.width() > 1920 || decoded.height() > 1080 {
            decoded.thumbnail(1920, 1080)
        } else {
            decoded
        };
        let mut png = Vec::new();
        scaled
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|err| BanterError::Storage(err.to_string()))?;
        self.account(user_id).await?;
        let path = self.root.join("banners").join(format!("{user_id}.png"));
        fs::write(&path, &png).map_err(|err| BanterError::Storage(err.to_string()))?;
        sqlx::query("UPDATE users SET banner_file = $1, banner_rev = banner_rev + 1 WHERE id = $2")
            .bind(format!("{user_id}.png"))
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    pub async fn clear_banner(&self, user_id: i64) -> Result<(), BanterError> {
        self.account(user_id).await?;
        let path = self.root.join("banners").join(format!("{user_id}.png"));
        if path.exists() {
            fs::remove_file(&path).map_err(|err| BanterError::Storage(err.to_string()))?;
        }
        sqlx::query("UPDATE users SET banner_file = NULL, banner_rev = 0 WHERE id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    pub async fn banner_bytes(&self, user_id: i64) -> Result<Vec<u8>, BanterError> {
        let file: Option<String> = sqlx::query_scalar("SELECT banner_file FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?
            .ok_or(BanterError::NotFound)?;
        let name = file.ok_or(BanterError::NotFound)?;
        let path = self.root.join("banners").join(name);
        fs::read(&path).map_err(|_| BanterError::NotFound)
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
        insert_everyone_role_tx(&mut tx, server_id).await?;
        place_server_on_rail_tx(&mut tx, owner_id, server_id, None).await?;
        tx.commit().await.map_err(db_err)?;
        self.server_home(owner_id, server_id).await
    }

    pub async fn create_invite(&self, user_id: i64, server_id: i64) -> Result<String, BanterError> {
        self.require_flag(user_id, server_id, PERM_CREATE_INVITE).await?;
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
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        sqlx::query("INSERT INTO memberships (server_id, user_id, nickname) VALUES ($1, $2, NULL)")
            .bind(server_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        place_server_on_rail_tx(&mut tx, user_id, server_id, None).await?;
        tx.commit().await.map_err(db_err)?;
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

    /// Private nested folder + server placement tree for the signed-in user's rail.
    pub async fn server_rail(&self, user_id: i64) -> Result<ServerRail, BanterError> {
        self.account(user_id).await?;
        self.ensure_rail_placements(user_id).await?;
        let folder_rows = sqlx::query(
            "SELECT id, parent_id, name, color, position, collapsed
             FROM server_folders WHERE user_id = $1
             ORDER BY position ASC, id ASC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        let folders = folder_rows
            .iter()
            .map(|row| ServerFolder {
                id: row.get("id"),
                parent_id: row.get("parent_id"),
                name: row.get("name"),
                color: row.get("color"),
                position: row.get("position"),
                collapsed: row.get("collapsed"),
            })
            .collect();
        let server_rows = sqlx::query(
            "SELECT s.id, s.name, s.owner_id, m.folder_id, m.position
             FROM server_folder_memberships m
             JOIN servers s ON s.id = m.server_id
             JOIN memberships mem ON mem.server_id = s.id AND mem.user_id = m.user_id
             WHERE m.user_id = $1
             ORDER BY m.position ASC, s.id ASC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        let servers = server_rows
            .iter()
            .map(|row| ServerRailServer {
                server: ServerSummary {
                    id: row.get("id"),
                    name: row.get("name"),
                    owner_id: row.get("owner_id"),
                },
                folder_id: row.get("folder_id"),
                position: row.get("position"),
            })
            .collect();
        Ok(ServerRail { folders, servers })
    }

    pub async fn create_server_folder(
        &self,
        user_id: i64,
        name: &str,
        parent_id: Option<i64>,
        color: Option<i64>,
    ) -> Result<ServerFolder, BanterError> {
        self.account(user_id).await?;
        let name = clean_label(name)?;
        let color = clean_folder_color(color.unwrap_or(0))?;
        let parents = self.folder_parent_map(user_id).await?;
        if parents.len() as i64 >= MAX_SERVER_FOLDERS {
            return Err(BanterError::InvalidName);
        }
        if let Some(parent) = parent_id {
            if !parents.contains_key(&parent) {
                return Err(BanterError::NotFound);
            }
        }
        let depth = child_depth(parent_id, &parents)?;
        if depth > MAX_SERVER_FOLDER_DEPTH {
            return Err(BanterError::InvalidName);
        }
        let position = next_folder_position(&self.pool, user_id, parent_id).await?;
        let row = sqlx::query(
            "INSERT INTO server_folders (user_id, parent_id, name, color, position, collapsed)
             VALUES ($1, $2, $3, $4, $5, FALSE)
             RETURNING id, parent_id, name, color, position, collapsed",
        )
        .bind(user_id)
        .bind(parent_id)
        .bind(&name)
        .bind(color)
        .bind(position)
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(map_server_folder(&row))
    }

    pub async fn update_server_folder(
        &self,
        user_id: i64,
        folder_id: i64,
        patch: ServerFolderPatch,
    ) -> Result<ServerFolder, BanterError> {
        let current = self.require_owned_folder(user_id, folder_id).await?;
        let name = match patch.name {
            Some(raw) => clean_label(&raw)?,
            None => current.name.clone(),
        };
        let color = match patch.color {
            Some(value) => clean_folder_color(value)?,
            None => current.color,
        };
        let collapsed = patch.collapsed.unwrap_or(current.collapsed);
        let parents = self.folder_parent_map(user_id).await?;
        let parent_id = match patch.parent_id {
            Some(value) => {
                if let Some(parent) = value {
                    if !parents.contains_key(&parent) {
                        return Err(BanterError::NotFound);
                    }
                }
                if would_cycle(folder_id, value, &parents) {
                    return Err(BanterError::InvalidName);
                }
                // Temporary map as if this folder already moved, then measure depth.
                let mut trial = parents.clone();
                trial.insert(folder_id, value);
                let depth = crate::folders::folder_depth(folder_id, &trial)?;
                if depth > MAX_SERVER_FOLDER_DEPTH {
                    return Err(BanterError::InvalidName);
                }
                // Also ensure every descendant stays within the depth cap.
                ensure_subtree_depth_ok(folder_id, &trial)?;
                value
            }
            None => current.parent_id,
        };
        let position = match patch.position {
            Some(value) if value >= 0 => value,
            Some(_) => return Err(BanterError::InvalidName),
            None if patch.parent_id.is_some() && parent_id != current.parent_id => {
                next_folder_position(&self.pool, user_id, parent_id).await?
            }
            None => current.position,
        };
        let row = sqlx::query(
            "UPDATE server_folders
             SET parent_id = $1, name = $2, color = $3, position = $4, collapsed = $5
             WHERE id = $6 AND user_id = $7
             RETURNING id, parent_id, name, color, position, collapsed",
        )
        .bind(parent_id)
        .bind(&name)
        .bind(color)
        .bind(position)
        .bind(collapsed)
        .bind(folder_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(map_server_folder(&row))
    }

    pub async fn delete_server_folder(&self, user_id: i64, folder_id: i64) -> Result<(), BanterError> {
        let folder = self.require_owned_folder(user_id, folder_id).await?;
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        // Promote direct child folders and servers to this folder's parent.
        sqlx::query(
            "UPDATE server_folders SET parent_id = $1 WHERE user_id = $2 AND parent_id = $3",
        )
        .bind(folder.parent_id)
        .bind(user_id)
        .bind(folder_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
        sqlx::query(
            "UPDATE server_folder_memberships SET folder_id = $1 WHERE user_id = $2 AND folder_id = $3",
        )
        .bind(folder.parent_id)
        .bind(user_id)
        .bind(folder_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
        let deleted = sqlx::query("DELETE FROM server_folders WHERE id = $1 AND user_id = $2")
            .bind(folder_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        if deleted.rows_affected() != 1 {
            return Err(BanterError::NotFound);
        }
        tx.commit().await.map_err(db_err)?;
        Ok(())
    }

    pub async fn move_server_to_folder(
        &self,
        user_id: i64,
        server_id: i64,
        folder_id: Option<i64>,
        position: i64,
    ) -> Result<ServerRailServer, BanterError> {
        self.require_member(user_id, server_id).await?;
        if position < 0 {
            return Err(BanterError::InvalidName);
        }
        if let Some(folder) = folder_id {
            self.require_owned_folder(user_id, folder).await?;
        }
        self.ensure_rail_placements(user_id).await?;
        let updated = sqlx::query(
            "UPDATE server_folder_memberships
             SET folder_id = $1, position = $2
             WHERE user_id = $3 AND server_id = $4",
        )
        .bind(folder_id)
        .bind(position)
        .bind(user_id)
        .bind(server_id)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        if updated.rows_affected() != 1 {
            // Membership exists but rail row missing — place then update.
            let mut tx = self.pool.begin().await.map_err(db_err)?;
            place_server_on_rail_tx(&mut tx, user_id, server_id, folder_id).await?;
            sqlx::query(
                "UPDATE server_folder_memberships
                 SET folder_id = $1, position = $2
                 WHERE user_id = $3 AND server_id = $4",
            )
            .bind(folder_id)
            .bind(position)
            .bind(user_id)
            .bind(server_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
            tx.commit().await.map_err(db_err)?;
        }
        let row = sqlx::query(
            "SELECT s.id, s.name, s.owner_id, m.folder_id, m.position
             FROM server_folder_memberships m
             JOIN servers s ON s.id = m.server_id
             WHERE m.user_id = $1 AND m.server_id = $2",
        )
        .bind(user_id)
        .bind(server_id)
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(ServerRailServer {
            server: ServerSummary {
                id: row.get("id"),
                name: row.get("name"),
                owner_id: row.get("owner_id"),
            },
            folder_id: row.get("folder_id"),
            position: row.get("position"),
        })
    }

    async fn require_owned_folder(&self, user_id: i64, folder_id: i64) -> Result<ServerFolder, BanterError> {
        let row = sqlx::query(
            "SELECT id, parent_id, name, color, position, collapsed
             FROM server_folders WHERE id = $1 AND user_id = $2",
        )
        .bind(folder_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        row.map(|row| map_server_folder(&row)).ok_or(BanterError::NotFound)
    }

    async fn folder_parent_map(&self, user_id: i64) -> Result<std::collections::HashMap<i64, Option<i64>>, BanterError> {
        let rows = sqlx::query("SELECT id, parent_id FROM server_folders WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(rows
            .iter()
            .map(|row| (row.get::<i64, _>("id"), row.get::<Option<i64>, _>("parent_id")))
            .collect())
    }

    async fn ensure_rail_placements(&self, user_id: i64) -> Result<(), BanterError> {
        let missing: Vec<i64> = sqlx::query_scalar(
            "SELECT m.server_id FROM memberships m
             LEFT JOIN server_folder_memberships r
               ON r.user_id = m.user_id AND r.server_id = m.server_id
             WHERE m.user_id = $1 AND r.server_id IS NULL
             ORDER BY m.server_id",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        if missing.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        for server_id in missing {
            place_server_on_rail_tx(&mut tx, user_id, server_id, None).await?;
        }
        tx.commit().await.map_err(db_err)?;
        Ok(())
    }

    pub async fn members(&self, user_id: i64, server_id: i64) -> Result<Vec<Member>, BanterError> {
        self.require_member(user_id, server_id).await?;
        let mut members = members_of(&self.pool, server_id).await?;
        attach_member_roles(&self.pool, server_id, &mut members).await?;
        Ok(members)
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
        self.require_flag(user_id, server_id, PERM_MANAGE_CHANNELS).await?;
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
        icon: Option<&str>,
    ) -> Result<Channel, BanterError> {
        let name = clean_label(name)?;
        let icon = crate::normalize_channel_icon(icon)?;
        self.require_flag(user_id, server_id, PERM_MANAGE_CHANNELS).await?;
        let category_server: Option<i64> =
            sqlx::query_scalar("SELECT server_id FROM categories WHERE id = $1")
                .bind(category_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(db_err)?;
        if category_server != Some(server_id) {
            return Err(BanterError::NotFound);
        }
        if kind.is_plugin() {
            return Err(BanterError::InvalidName);
        }
        let position: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM channels WHERE category_id = $1",
        )
        .bind(category_id)
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO channels (server_id, category_id, name, kind, position, icon)
             VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
        )
        .bind(server_id)
        .bind(category_id)
        .bind(&name)
        .bind(kind.as_str())
        .bind(position)
        .bind(&icon)
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        if kind.is_board() {
            insert_empty_board_scene(&self.pool, id).await?;
        }
        Ok(Channel {
            id,
            server_id,
            category_id,
            name,
            kind,
            position,
            icon,
            created_by_plugin: None,
        })
    }

    /// Rename a channel and set or clear its optional Lucide icon.
    pub async fn update_channel(
        &self,
        user_id: i64,
        server_id: i64,
        channel_id: i64,
        name: &str,
        icon: Option<&str>,
    ) -> Result<Channel, BanterError> {
        let name = clean_label(name)?;
        let icon = crate::normalize_channel_icon(icon)?;
        self.require_flag(user_id, server_id, PERM_MANAGE_CHANNELS).await?;
        let existing = channel_by_id(&self.pool, channel_id).await?;
        if existing.server_id != server_id {
            return Err(BanterError::NotFound);
        }
        let result = sqlx::query(
            "UPDATE channels SET name = $1, icon = $2 WHERE id = $3 AND server_id = $4",
        )
        .bind(&name)
        .bind(&icon)
        .bind(channel_id)
        .bind(server_id)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        if result.rows_affected() == 0 {
            return Err(BanterError::NotFound);
        }
        Ok(Channel {
            id: existing.id,
            server_id: existing.server_id,
            category_id: existing.category_id,
            name,
            kind: existing.kind,
            position: existing.position,
            icon,
            created_by_plugin: existing.created_by_plugin,
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

    /// Members may read the guild command prefix and aliases.
    pub async fn command_settings(
        &self,
        user_id: i64,
        server_id: i64,
    ) -> Result<ServerCommandSettings, BanterError> {
        self.require_member(user_id, server_id).await?;
        load_command_settings(&self.pool, server_id).await
    }

    /// Manage Channels (or Administrator / owner) may change the guild prefix and aliases.
    pub async fn set_command_settings(
        &self,
        user_id: i64,
        server_id: i64,
        prefix: &str,
        aliases: Vec<CommandAlias>,
    ) -> Result<ServerCommandSettings, BanterError> {
        self.require_flag(user_id, server_id, PERM_MANAGE_CHANNELS).await?;
        crate::commands::validate_command_prefix(prefix)?;
        let aliases = crate::commands::normalize_command_aliases(aliases)?;
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let updated: Option<i64> = sqlx::query_scalar(
            "UPDATE servers SET command_prefix = $1 WHERE id = $2 RETURNING id",
        )
        .bind(prefix)
        .bind(server_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?;
        if updated.is_none() {
            return Err(BanterError::NotFound);
        }
        sqlx::query("DELETE FROM server_command_aliases WHERE server_id = $1")
            .bind(server_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        for entry in &aliases {
            sqlx::query(
                "INSERT INTO server_command_aliases (server_id, alias, target) VALUES ($1, $2, $3)",
            )
            .bind(server_id)
            .bind(&entry.alias)
            .bind(&entry.target)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        }
        tx.commit().await.map_err(db_err)?;
        Ok(ServerCommandSettings {
            prefix: prefix.to_string(),
            aliases,
        })
    }

    /// Voice channels in this server, including empty ones. Caller must be a member.
    pub async fn voice_channel_ids(&self, user_id: i64, server_id: i64) -> Result<Vec<i64>, BanterError> {
        self.require_member(user_id, server_id).await?;
        sqlx::query_scalar(
            "SELECT id FROM channels WHERE server_id = $1 AND kind = 'voice' ORDER BY position, id",
        )
        .bind(server_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)
    }

    pub(crate) async fn member_user_ids(&self, server_id: i64) -> Result<Vec<i64>, BanterError> {
        sqlx::query_scalar("SELECT user_id FROM memberships WHERE server_id = $1")
            .bind(server_id)
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)
    }

    pub async fn send_message(
        &self,
        user_id: i64,
        target: ChatTarget,
        body: &str,
        reply_to: Option<i64>,
        files: &[(&str, &[u8])],
        media_asset_ids: &[i64],
    ) -> Result<Message, BanterError> {
        let body = body.trim().to_string();
        if body.chars().count() > MAX_MESSAGE_CHARS {
            return Err(BanterError::MessageTooLong);
        }
        if body.is_empty() && files.is_empty() && media_asset_ids.is_empty() {
            return Err(BanterError::EmptyMessage);
        }
        for (_, bytes) in files {
            if bytes.is_empty() || bytes.len() > MAX_ATTACHMENT_BYTES {
                return Err(BanterError::FileTooLarge);
            }
        }
        let media_rows = self
            .resolve_send_media(user_id, target, media_asset_ids)
            .await?;
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
            for (sort_order, (asset_id, kind)) in media_rows.iter().enumerate() {
                sqlx::query(
                    "INSERT INTO message_media (message_id, asset_id, kind, sort_order)
                     VALUES ($1, $2, $3, $4)",
                )
                .bind(message_id)
                .bind(asset_id)
                .bind(kind.as_str())
                .bind(sort_order as i64)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
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

    pub(crate) async fn require_member(&self, user_id: i64, server_id: i64) -> Result<(), BanterError> {
        if is_member(&self.pool, user_id, server_id).await? {
            Ok(())
        } else {
            Err(BanterError::NotMember)
        }
    }

    pub(crate) async fn require_flag(&self, user_id: i64, server_id: i64, flag: i64) -> Result<(), BanterError> {
        if !is_member(&self.pool, user_id, server_id).await? {
            return Err(BanterError::NotMember);
        }
        let bits = permission_bits(&self.pool, user_id, server_id).await?;
        if has_permission(bits, flag) {
            Ok(())
        } else {
            Err(BanterError::NotOwner)
        }
    }

    pub async fn roles(&self, user_id: i64, server_id: i64) -> Result<Vec<GuildRole>, BanterError> {
        self.require_member(user_id, server_id).await?;
        roles_on_server(&self.pool, server_id).await
    }

    pub async fn create_role(
        &self,
        user_id: i64,
        server_id: i64,
        name: &str,
        color: i64,
        hoist: bool,
        permissions: i64,
    ) -> Result<GuildRole, BanterError> {
        let name = clean_label(name)?;
        if name == EVERYONE_NAME {
            return Err(BanterError::InvalidName);
        }
        let color = clean_role_color(color)?;
        let permissions = permissions & ROLE_PERMISSION_MASK;
        self.require_flag(user_id, server_id, PERM_MANAGE_ROLES).await?;
        let actor_bits = permission_bits(&self.pool, user_id, server_id).await?;
        if !grants_only_held(actor_bits, permissions) {
            return Err(BanterError::RoleHierarchy);
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roles WHERE server_id = $1")
            .bind(server_id)
            .fetch_one(&self.pool)
            .await
            .map_err(db_err)?;
        if count >= MAX_GUILD_ROLES {
            return Err(BanterError::TooManyRoles);
        }
        let positions = role_positions(&self.pool, server_id).await?;
        let ceiling = actor_ceiling(&self.pool, user_id, server_id).await?;
        let Some(position) = next_role_position(&positions, ceiling) else {
            return Err(BanterError::RoleHierarchy);
        };
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO roles (server_id, name, color, position, hoist, permissions, everyone)
             VALUES ($1, $2, $3, $4, $5, $6, FALSE) RETURNING id",
        )
        .bind(server_id)
        .bind(&name)
        .bind(color)
        .bind(position)
        .bind(hoist)
        .bind(permissions)
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        role_by_id(&self.pool, server_id, id).await
    }

    pub async fn update_role(
        &self,
        user_id: i64,
        server_id: i64,
        role_id: i64,
        name: &str,
        color: i64,
        hoist: bool,
        permissions: i64,
    ) -> Result<GuildRole, BanterError> {
        let color = clean_role_color(color)?;
        let permissions = permissions & ROLE_PERMISSION_MASK;
        self.require_flag(user_id, server_id, PERM_MANAGE_ROLES).await?;
        let role = role_by_id(&self.pool, server_id, role_id).await?;
        let ceiling = actor_ceiling(&self.pool, user_id, server_id).await?;
        if !role.everyone && role.position >= ceiling {
            return Err(BanterError::RoleHierarchy);
        }
        let actor_bits = permission_bits(&self.pool, user_id, server_id).await?;
        if !grants_only_held(actor_bits, permissions) {
            return Err(BanterError::RoleHierarchy);
        }
        let name = if role.everyone {
            EVERYONE_NAME.to_string()
        } else {
            let name = clean_label(name)?;
            if name == EVERYONE_NAME {
                return Err(BanterError::InvalidName);
            }
            name
        };
        sqlx::query(
            "UPDATE roles SET name = $1, color = $2, hoist = $3, permissions = $4
             WHERE id = $5 AND server_id = $6",
        )
        .bind(&name)
        .bind(color)
        .bind(hoist && !role.everyone)
        .bind(permissions)
        .bind(role_id)
        .bind(server_id)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        role_by_id(&self.pool, server_id, role_id).await
    }

    pub async fn delete_role(&self, user_id: i64, server_id: i64, role_id: i64) -> Result<(), BanterError> {
        self.require_flag(user_id, server_id, PERM_MANAGE_ROLES).await?;
        let role = role_by_id(&self.pool, server_id, role_id).await?;
        if role.everyone {
            return Err(BanterError::InvalidName);
        }
        let ceiling = actor_ceiling(&self.pool, user_id, server_id).await?;
        if role.position >= ceiling {
            return Err(BanterError::RoleHierarchy);
        }
        sqlx::query("DELETE FROM roles WHERE id = $1 AND server_id = $2")
            .bind(role_id)
            .bind(server_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    pub async fn set_member_roles(
        &self,
        user_id: i64,
        server_id: i64,
        target_id: i64,
        role_ids: &[i64],
    ) -> Result<(), BanterError> {
        self.require_flag(user_id, server_id, PERM_MANAGE_ROLES).await?;
        if !is_member(&self.pool, target_id, server_id).await? {
            return Err(BanterError::NotMember);
        }
        let ceiling = actor_ceiling(&self.pool, user_id, server_id).await?;
        let roles = roles_on_server(&self.pool, server_id).await?;
        let mut chosen = Vec::new();
        for role_id in role_ids {
            let Some(role) = roles.iter().find(|role| role.id == *role_id) else {
                return Err(BanterError::NotFound);
            };
            if role.everyone {
                continue;
            }
            if role.position >= ceiling {
                return Err(BanterError::RoleHierarchy);
            }
            if !chosen.contains(role_id) {
                chosen.push(*role_id);
            }
        }
        let actor_bits = permission_bits(&self.pool, user_id, server_id).await?;
        for role_id in &chosen {
            let permissions = roles.iter().find(|role| role.id == *role_id).map(|role| role.permissions).unwrap_or(0);
            if !grants_only_held(actor_bits, permissions) {
                return Err(BanterError::RoleHierarchy);
            }
        }
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        sqlx::query("DELETE FROM member_roles WHERE server_id = $1 AND user_id = $2")
            .bind(server_id)
            .bind(target_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        for role_id in chosen {
            sqlx::query(
                "INSERT INTO member_roles (server_id, user_id, role_id) VALUES ($1, $2, $3)",
            )
            .bind(server_id)
            .bind(target_id)
            .bind(role_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        }
        tx.commit().await.map_err(db_err)?;
        Ok(())
    }

    pub async fn table_counts(&self) -> Result<(i64, i64, i64, i64), BanterError> {
        let users: i64 = sqlx::query_scalar("SELECT COUNT(*)::BIGINT FROM users")
            .fetch_one(&self.pool)
            .await
            .map_err(db_err)?;
        let servers: i64 = sqlx::query_scalar("SELECT COUNT(*)::BIGINT FROM servers")
            .fetch_one(&self.pool)
            .await
            .map_err(db_err)?;
        let channels: i64 = sqlx::query_scalar("SELECT COUNT(*)::BIGINT FROM channels")
            .fetch_one(&self.pool)
            .await
            .map_err(db_err)?;
        let messages: i64 = sqlx::query_scalar("SELECT COUNT(*)::BIGINT FROM messages")
            .fetch_one(&self.pool)
            .await
            .map_err(db_err)?;
        Ok((users, servers, channels, messages))
    }

    pub fn pool_usage(&self) -> (i64, i64) {
        (self.pool.size() as i64, self.pool.num_idle() as i64)
    }
}

pub(crate) async fn account_by_id(pool: &PgPool, id: i64) -> Result<Account, BanterError> {
    let row = sqlx::query(
        "SELECT id, username, display_name, avatar_file, avatar_rev, bot FROM users WHERE id = $1",
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
        "SELECT id, username, display_name, avatar_file, avatar_rev, bot FROM users WHERE username = $1",
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
        bot: row.get("bot"),
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

async fn share_server(pool: &PgPool, a: i64, b: i64) -> Result<bool, BanterError> {
    let found: Option<bool> = sqlx::query_scalar(
        "SELECT TRUE FROM memberships m1
         JOIN memberships m2 ON m1.server_id = m2.server_id
         WHERE m1.user_id = $1 AND m2.user_id = $2
         LIMIT 1",
    )
    .bind(a)
    .bind(b)
    .fetch_optional(pool)
    .await
    .map_err(db_err)?;
    Ok(found.is_some())
}

async fn member_nickname(pool: &PgPool, server_id: i64, user_id: i64) -> Result<Option<String>, BanterError> {
    let row: Option<Option<String>> = sqlx::query_scalar(
        "SELECT nickname FROM memberships WHERE server_id = $1 AND user_id = $2",
    )
    .bind(server_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(db_err)?;
    Ok(row.flatten())
}

async fn load_user_profile(
    pool: &PgPool,
    user_id: i64,
    nickname: Option<String>,
    is_self: bool,
    is_friend: bool,
) -> Result<UserProfile, BanterError> {
    let row = sqlx::query(
        "SELECT id, username, display_name, avatar_file, avatar_rev,
                about_me, pronouns, custom_status, created_at,
                banner_file, banner_rev, theme_primary, theme_secondary,
                nameplate_id, avatar_decoration_id, display_name_style_id,
                profile_effect_id, profile_frame_id, badge_ids
         FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(db_err)?
    .ok_or(BanterError::NotFound)?;

    let theme_primary = row
        .get::<Option<i64>, _>("theme_primary")
        .map(|v| v as u32)
        .unwrap_or(DEFAULT_THEME_PRIMARY);
    let theme_secondary = row
        .get::<Option<i64>, _>("theme_secondary")
        .map(|v| v as u32)
        .unwrap_or(DEFAULT_THEME_SECONDARY);
    let badge_ids_raw: String = row.try_get("badge_ids").unwrap_or_else(|_| "[]".into());
    let badges = resolve_badges(&badge_ids_raw)?;

    Ok(UserProfile {
        id: row.get("id"),
        username: row.get("username"),
        display_name: row.get("display_name"),
        has_avatar: row.get::<Option<String>, _>("avatar_file").is_some(),
        avatar_rev: row.get("avatar_rev"),
        nickname,
        is_self,
        is_friend,
        about_me: row.get("about_me"),
        pronouns: row.get("pronouns"),
        custom_status: row.get("custom_status"),
        member_since: row.get("created_at"),
        has_banner: row.get::<Option<String>, _>("banner_file").is_some(),
        banner_rev: row.get("banner_rev"),
        theme_primary,
        theme_secondary,
        nameplate_id: row.get("nameplate_id"),
        avatar_decoration_id: row.get("avatar_decoration_id"),
        display_name_style_id: row.get("display_name_style_id"),
        profile_effect_id: row.get("profile_effect_id"),
        profile_frame_id: row.get("profile_frame_id"),
        widgets: list_widgets(pool, user_id).await?,
        connections: list_connections(pool, user_id).await?,
        badges,
        activity: list_activity(pool, user_id).await?,
        wishlist: list_wishlist(pool, user_id).await?,
        roles: Vec::new(),
    })
}

async fn apply_cosmetic(
    pool: &PgPool,
    user_id: i64,
    column: &str,
    value: Option<&str>,
    kind: CosmeticKind,
) -> Result<(), BanterError> {
    let Some(raw) = value else {
        return Ok(());
    };
    let stored = if raw.trim().is_empty() {
        None
    } else if cosmetic_exists(kind, raw.trim()) {
        Some(raw.trim().to_string())
    } else {
        return Err(BanterError::InvalidProfile);
    };
    // Columns are fixed call-site literals; never pass untrusted names here.
    let sql = format!("UPDATE users SET {column} = $1 WHERE id = $2");
    sqlx::query(&sql)
        .bind(stored)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(db_err)?;
    Ok(())
}

async fn replace_widgets(pool: &PgPool, user_id: i64, widgets: &[ProfileWidgetInput]) -> Result<(), BanterError> {
    if widgets.len() > MAX_PROFILE_WIDGETS {
        return Err(BanterError::InvalidProfile);
    }
    sqlx::query("DELETE FROM profile_widgets WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(db_err)?;
    for (position, widget) in widgets.iter().enumerate() {
        let title = clean_label(&widget.title)?;
        let payload = serde_json::to_string(&widget.payload)
            .map_err(|err| BanterError::Storage(err.to_string()))?;
        if payload.len() > 8 * 1024 {
            return Err(BanterError::InvalidProfile);
        }
        let kind = widget_kind_key(widget.kind);
        sqlx::query(
            "INSERT INTO profile_widgets (user_id, kind, title, payload, position)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(user_id)
        .bind(kind)
        .bind(&title)
        .bind(&payload)
        .bind(position as i64)
        .execute(pool)
        .await
        .map_err(db_err)?;
    }
    Ok(())
}

async fn replace_connections(
    pool: &PgPool,
    user_id: i64,
    connections: &[ProfileConnectionInput],
) -> Result<(), BanterError> {
    if connections.len() > MAX_PROFILE_CONNECTIONS {
        return Err(BanterError::InvalidProfile);
    }
    sqlx::query("DELETE FROM profile_connections WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(db_err)?;
    for (position, connection) in connections.iter().enumerate() {
        let kind = clean_connection_kind(&connection.kind)?;
        let name = clean_label(&connection.name)?;
        let url = clean_connection_url(&connection.url)?;
        sqlx::query(
            "INSERT INTO profile_connections (user_id, kind, name, url, position)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(user_id)
        .bind(&kind)
        .bind(&name)
        .bind(&url)
        .bind(position as i64)
        .execute(pool)
        .await
        .map_err(db_err)?;
    }
    Ok(())
}

async fn list_widgets(pool: &PgPool, user_id: i64) -> Result<Vec<ProfileWidget>, BanterError> {
    let rows = sqlx::query(
        "SELECT id, kind, title, payload FROM profile_widgets
         WHERE user_id = $1 ORDER BY position ASC, id ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(db_err)?;
    let mut out = Vec::new();
    for row in rows {
        let kind = parse_widget_kind(&row.get::<String, _>("kind")).ok_or(BanterError::InvalidProfile)?;
        let payload: serde_json::Value = serde_json::from_str(&row.get::<String, _>("payload"))
            .map_err(|err| BanterError::Storage(err.to_string()))?;
        out.push(ProfileWidget {
            id: row.get("id"),
            kind,
            title: row.get("title"),
            payload,
        });
    }
    Ok(out)
}

async fn list_connections(pool: &PgPool, user_id: i64) -> Result<Vec<ProfileConnection>, BanterError> {
    let rows = sqlx::query(
        "SELECT id, kind, name, url FROM profile_connections
         WHERE user_id = $1 ORDER BY position ASC, id ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(db_err)?;
    Ok(rows
        .iter()
        .map(|row| ProfileConnection {
            id: row.get("id"),
            kind: row.get("kind"),
            name: row.get("name"),
            url: row.get("url"),
        })
        .collect())
}

fn resolve_badges(raw: &str) -> Result<Vec<ProfileBadge>, BanterError> {
    let ids: Vec<String> = serde_json::from_str(raw).unwrap_or_default();
    let mut out = Vec::new();
    for id in ids {
        if let Some(item) = badge_item(&id) {
            out.push(ProfileBadge {
                id: item.id,
                label: item.label,
                color: item.color,
                shape: item.shape,
            });
        }
    }
    Ok(out)
}

async fn replace_badges(pool: &PgPool, user_id: i64, badge_ids: &[String]) -> Result<(), BanterError> {
    if badge_ids.len() > MAX_EQUIPPED_BADGES {
        return Err(BanterError::InvalidProfile);
    }
    let mut unique = Vec::new();
    for id in badge_ids {
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        if !badge_exists(id) {
            return Err(BanterError::InvalidProfile);
        }
        if !unique.iter().any(|existing| existing == id) {
            unique.push(id.to_string());
        }
    }
    let json = serde_json::to_string(&unique).map_err(|err| BanterError::Storage(err.to_string()))?;
    sqlx::query("UPDATE users SET badge_ids = $1 WHERE id = $2")
        .bind(json)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(db_err)?;
    Ok(())
}

async fn replace_activity(
    pool: &PgPool,
    user_id: i64,
    activity: &[ProfileActivityInput],
) -> Result<(), BanterError> {
    if activity.len() > MAX_PROFILE_ACTIVITY {
        return Err(BanterError::InvalidProfile);
    }
    sqlx::query("DELETE FROM profile_activity WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(db_err)?;
    for (position, entry) in activity.iter().enumerate() {
        let kind = clean_activity_kind(&entry.kind)?;
        let title = clean_label(&entry.title)?;
        let detail = clean_optional_text(&entry.detail, 200)?.unwrap_or_default();
        sqlx::query(
            "INSERT INTO profile_activity (user_id, kind, title, detail, position)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(user_id)
        .bind(&kind)
        .bind(&title)
        .bind(&detail)
        .bind(position as i64)
        .execute(pool)
        .await
        .map_err(db_err)?;
    }
    Ok(())
}

async fn replace_wishlist(
    pool: &PgPool,
    user_id: i64,
    wishlist: &[ProfileWishlistInput],
) -> Result<(), BanterError> {
    if wishlist.len() > MAX_PROFILE_WISHLIST {
        return Err(BanterError::InvalidProfile);
    }
    sqlx::query("DELETE FROM profile_wishlist WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(db_err)?;
    for (position, entry) in wishlist.iter().enumerate() {
        let title = clean_label(&entry.title)?;
        let tag = clean_optional_text(&entry.tag, 40)?.unwrap_or_default();
        sqlx::query(
            "INSERT INTO profile_wishlist (user_id, title, tag, position)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(user_id)
        .bind(&title)
        .bind(&tag)
        .bind(position as i64)
        .execute(pool)
        .await
        .map_err(db_err)?;
    }
    Ok(())
}

async fn list_activity(pool: &PgPool, user_id: i64) -> Result<Vec<ProfileActivity>, BanterError> {
    let rows = sqlx::query(
        "SELECT id, kind, title, detail FROM profile_activity
         WHERE user_id = $1 ORDER BY position ASC, id ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(db_err)?;
    Ok(rows
        .iter()
        .map(|row| ProfileActivity {
            id: row.get("id"),
            kind: row.get("kind"),
            title: row.get("title"),
            detail: row.get("detail"),
        })
        .collect())
}

async fn list_wishlist(pool: &PgPool, user_id: i64) -> Result<Vec<ProfileWishlistItem>, BanterError> {
    let rows = sqlx::query(
        "SELECT id, title, tag FROM profile_wishlist
         WHERE user_id = $1 ORDER BY position ASC, id ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(db_err)?;
    Ok(rows
        .iter()
        .map(|row| ProfileWishlistItem {
            id: row.get("id"),
            title: row.get("title"),
            tag: row.get("tag"),
        })
        .collect())
}

fn clean_activity_kind(raw: &str) -> Result<String, BanterError> {
    let kind = raw.trim().to_lowercase();
    match kind.as_str() {
        "custom" | "playing" | "listening" | "watching" => Ok(kind),
        _ => Err(BanterError::InvalidProfile),
    }
}

fn widget_kind_key(kind: ProfileWidgetKind) -> &'static str {
    match kind {
        ProfileWidgetKind::GamesInRotation => "games_in_rotation",
        ProfileWidgetKind::FavoriteGame => "favorite_game",
        ProfileWidgetKind::GamesILike => "games_i_like",
    }
}

fn parse_widget_kind(raw: &str) -> Option<ProfileWidgetKind> {
    match raw {
        "games_in_rotation" => Some(ProfileWidgetKind::GamesInRotation),
        "favorite_game" => Some(ProfileWidgetKind::FavoriteGame),
        "games_i_like" => Some(ProfileWidgetKind::GamesILike),
        _ => None,
    }
}

fn clean_optional_text(raw: &str, max_chars: usize) -> Result<Option<String>, BanterError> {
    let text = raw.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if text.chars().count() > max_chars || text.chars().any(|c| c.is_control()) {
        return Err(BanterError::InvalidProfile);
    }
    Ok(Some(text.to_string()))
}

fn clean_connection_kind(raw: &str) -> Result<String, BanterError> {
    let kind = raw.trim().to_lowercase();
    match kind.as_str() {
        "website" | "github" | "steam" | "twitch" | "youtube" | "twitter" | "other" => Ok(kind),
        _ => Err(BanterError::InvalidProfile),
    }
}

fn clean_connection_url(raw: &str) -> Result<String, BanterError> {
    let url = raw.trim();
    let count = url.chars().count();
    if !(1..=500).contains(&count) || url.chars().any(|c| c.is_control()) {
        return Err(BanterError::InvalidProfile);
    }
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(BanterError::InvalidProfile);
    }
    Ok(url.to_string())
}

fn ordered_pair(a: i64, b: i64) -> (i64, i64) {
    if a < b { (a, b) } else { (b, a) }
}

async fn insert_everyone_role_tx(tx: &mut Transaction<'_, Postgres>, server_id: i64) -> Result<(), BanterError> {
    sqlx::query(
        "INSERT INTO roles (server_id, name, color, position, hoist, permissions, everyone)
         VALUES ($1, $2, 0, 0, FALSE, $3, TRUE)",
    )
    .bind(server_id)
    .bind(EVERYONE_NAME)
    .bind(EVERYONE_PERMISSIONS)
    .execute(&mut **tx)
    .await
    .map_err(db_err)?;
    Ok(())
}

fn map_role(row: &sqlx::postgres::PgRow) -> GuildRole {
    GuildRole {
        id: row.get("id"),
        server_id: row.get("server_id"),
        name: row.get("name"),
        color: row.get("color"),
        position: row.get("position"),
        hoist: row.get("hoist"),
        permissions: row.get("permissions"),
        everyone: row.get("everyone"),
    }
}

fn clean_role_color(color: i64) -> Result<i64, BanterError> {
    if role_color_ok(color) {
        Ok(color)
    } else {
        Err(BanterError::InvalidName)
    }
}

fn clean_folder_color(color: i64) -> Result<i64, BanterError> {
    if folder_color_ok(color) {
        Ok(color)
    } else {
        Err(BanterError::InvalidName)
    }
}

fn map_server_folder(row: &sqlx::postgres::PgRow) -> ServerFolder {
    ServerFolder {
        id: row.get("id"),
        parent_id: row.get("parent_id"),
        name: row.get("name"),
        color: row.get("color"),
        position: row.get("position"),
        collapsed: row.get("collapsed"),
    }
}

fn ensure_subtree_depth_ok(
    root_id: i64,
    parent_of: &std::collections::HashMap<i64, Option<i64>>,
) -> Result<(), BanterError> {
    let mut children: std::collections::HashMap<i64, Vec<i64>> = std::collections::HashMap::new();
    for (id, parent) in parent_of {
        if let Some(parent) = parent {
            children.entry(*parent).or_default().push(*id);
        }
    }
    let mut stack = vec![root_id];
    while let Some(id) = stack.pop() {
        let depth = crate::folders::folder_depth(id, parent_of)?;
        if depth > MAX_SERVER_FOLDER_DEPTH {
            return Err(BanterError::InvalidName);
        }
        if let Some(kids) = children.get(&id) {
            stack.extend(kids.iter().copied());
        }
    }
    Ok(())
}

async fn next_folder_position(
    pool: &PgPool,
    user_id: i64,
    parent_id: Option<i64>,
) -> Result<i64, BanterError> {
    let max: Option<i64> = match parent_id {
        Some(parent) => {
            sqlx::query_scalar(
                "SELECT MAX(position) FROM server_folders WHERE user_id = $1 AND parent_id = $2",
            )
            .bind(user_id)
            .bind(parent)
            .fetch_one(pool)
            .await
            .map_err(db_err)?
        }
        None => sqlx::query_scalar(
            "SELECT MAX(position) FROM server_folders WHERE user_id = $1 AND parent_id IS NULL",
        )
        .bind(user_id)
        .fetch_one(pool)
        .await
        .map_err(db_err)?,
    };
    Ok(max.unwrap_or(-1).saturating_add(1))
}

async fn next_rail_server_position(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
    folder_id: Option<i64>,
) -> Result<i64, BanterError> {
    let max: Option<i64> = match folder_id {
        Some(folder) => {
            sqlx::query_scalar(
                "SELECT MAX(position) FROM server_folder_memberships WHERE user_id = $1 AND folder_id = $2",
            )
            .bind(user_id)
            .bind(folder)
            .fetch_one(&mut **tx)
            .await
            .map_err(db_err)?
        }
        None => sqlx::query_scalar(
            "SELECT MAX(position) FROM server_folder_memberships WHERE user_id = $1 AND folder_id IS NULL",
        )
        .bind(user_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(db_err)?,
    };
    Ok(max.unwrap_or(-1).saturating_add(1))
}

async fn place_server_on_rail_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: i64,
    server_id: i64,
    folder_id: Option<i64>,
) -> Result<(), BanterError> {
    let position = next_rail_server_position(tx, user_id, folder_id).await?;
    sqlx::query(
        "INSERT INTO server_folder_memberships (user_id, server_id, folder_id, position)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (user_id, server_id) DO NOTHING",
    )
    .bind(user_id)
    .bind(server_id)
    .bind(folder_id)
    .bind(position)
    .execute(&mut **tx)
    .await
    .map_err(db_err)?;
    Ok(())
}

async fn roles_on_server(pool: &PgPool, server_id: i64) -> Result<Vec<GuildRole>, BanterError> {
    let rows = sqlx::query(
        "SELECT id, server_id, name, color, position, hoist, permissions, everyone
         FROM roles WHERE server_id = $1
         ORDER BY position DESC, id ASC",
    )
    .bind(server_id)
    .fetch_all(pool)
    .await
    .map_err(db_err)?;
    Ok(rows.iter().map(map_role).collect())
}

async fn roles_for_member(pool: &PgPool, server_id: i64, user_id: i64) -> Result<Vec<GuildRole>, BanterError> {
    let roles = roles_on_server(pool, server_id).await?;
    let held: Vec<i64> = sqlx::query_scalar(
        "SELECT role_id FROM member_roles WHERE server_id = $1 AND user_id = $2",
    )
    .bind(server_id)
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(db_err)?;
    Ok(roles
        .into_iter()
        .filter(|role| role.everyone || held.contains(&role.id))
        .collect())
}

async fn role_by_id(pool: &PgPool, server_id: i64, role_id: i64) -> Result<GuildRole, BanterError> {
    let row = sqlx::query(
        "SELECT id, server_id, name, color, position, hoist, permissions, everyone
         FROM roles WHERE id = $1 AND server_id = $2",
    )
    .bind(role_id)
    .bind(server_id)
    .fetch_optional(pool)
    .await
    .map_err(db_err)?
    .ok_or(BanterError::NotFound)?;
    Ok(map_role(&row))
}

async fn role_positions(pool: &PgPool, server_id: i64) -> Result<Vec<i64>, BanterError> {
    sqlx::query_scalar("SELECT position FROM roles WHERE server_id = $1")
        .bind(server_id)
        .fetch_all(pool)
        .await
        .map_err(db_err)
}

async fn server_owner_id(pool: &PgPool, server_id: i64) -> Result<i64, BanterError> {
    sqlx::query_scalar("SELECT owner_id FROM servers WHERE id = $1")
        .bind(server_id)
        .fetch_optional(pool)
        .await
        .map_err(db_err)?
        .ok_or(BanterError::NotFound)
}

async fn permission_bits(pool: &PgPool, user_id: i64, server_id: i64) -> Result<i64, BanterError> {
    if server_owner_id(pool, server_id).await? == user_id {
        return Ok(PERM_ADMINISTRATOR);
    }
    let bits: Vec<i64> = sqlx::query_scalar(
        "SELECT permissions FROM roles
         WHERE server_id = $1 AND (
            everyone OR id IN (SELECT role_id FROM member_roles WHERE server_id = $1 AND user_id = $2)
         )",
    )
    .bind(server_id)
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(db_err)?;
    Ok(bits.into_iter().fold(0, |acc, bit| acc | bit))
}

async fn actor_ceiling(pool: &PgPool, user_id: i64, server_id: i64) -> Result<i64, BanterError> {
    if server_owner_id(pool, server_id).await? == user_id {
        return Ok(i64::MAX);
    }
    let top: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(position), 0) FROM roles
         WHERE server_id = $1 AND (
            everyone OR id IN (SELECT role_id FROM member_roles WHERE server_id = $1 AND user_id = $2)
         )",
    )
    .bind(server_id)
    .bind(user_id)
    .fetch_one(pool)
    .await
    .map_err(db_err)?;
    Ok(top)
}

pub(crate) async fn is_member(pool: &PgPool, user_id: i64, server_id: i64) -> Result<bool, BanterError> {
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
        "SELECT u.id, u.username, u.display_name, m.nickname, u.avatar_file, u.avatar_rev, u.bot
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
                bot: row.get("bot"),
                roles: Vec::new(),
            }
        })
        .collect())
}

async fn attach_member_roles(pool: &PgPool, server_id: i64, members: &mut [Member]) -> Result<(), BanterError> {
    let roles = roles_on_server(pool, server_id).await?;
    let link_rows = sqlx::query("SELECT user_id, role_id FROM member_roles WHERE server_id = $1")
        .bind(server_id)
        .fetch_all(pool)
        .await
        .map_err(db_err)?;
    let links: Vec<(i64, i64)> = link_rows
        .iter()
        .map(|row| (row.get("user_id"), row.get("role_id")))
        .collect();
    for member in members {
        member.roles = roles
            .iter()
            .filter(|role| {
                role.everyone || links.iter().any(|(user_id, role_id)| *user_id == member.user_id && *role_id == role.id)
            })
            .cloned()
            .collect();
    }
    Ok(())
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

async fn load_command_settings(
    pool: &PgPool,
    server_id: i64,
) -> Result<ServerCommandSettings, BanterError> {
    let prefix: Option<String> =
        sqlx::query_scalar("SELECT command_prefix FROM servers WHERE id = $1")
            .bind(server_id)
            .fetch_optional(pool)
            .await
            .map_err(db_err)?;
    let Some(prefix) = prefix else {
        return Err(BanterError::NotFound);
    };
    let rows = sqlx::query(
        "SELECT alias, target FROM server_command_aliases WHERE server_id = $1 ORDER BY alias",
    )
    .bind(server_id)
    .fetch_all(pool)
    .await
    .map_err(db_err)?;
    let aliases = rows
        .into_iter()
        .map(|row| CommandAlias {
            alias: row.get("alias"),
            target: row.get("target"),
        })
        .collect();
    Ok(ServerCommandSettings { prefix, aliases })
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
            "SELECT id, server_id, category_id, name, kind, position, icon, created_by_plugin
             FROM channels WHERE category_id = $1 ORDER BY position, id",
        )
        .bind(category.id)
        .fetch_all(pool)
        .await
        .map_err(db_err)?;
        let channels = channel_rows.iter().map(channel_from_row).collect();
        layouts.push(CategoryLayout { category, channels });
    }
    Ok(layouts)
}

fn channel_from_row(row: &sqlx::postgres::PgRow) -> Channel {
    Channel {
        id: row.get("id"),
        server_id: row.get("server_id"),
        category_id: row.get("category_id"),
        name: row.get("name"),
        kind: ChannelKind::parse(&row.get::<String, _>("kind")).unwrap_or(ChannelKind::Text),
        position: row.get("position"),
        icon: row.get("icon"),
        created_by_plugin: row.get("created_by_plugin"),
    }
}

pub(crate) async fn channel_by_id(pool: &PgPool, id: i64) -> Result<Channel, BanterError> {
    let row = sqlx::query(
        "SELECT id, server_id, category_id, name, kind, position, icon, created_by_plugin FROM channels WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(db_err)?
    .ok_or(BanterError::NotFound)?;
    Ok(channel_from_row(&row))
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
                "SELECT id, server_id, category_id, name, kind, position, icon, created_by_plugin FROM channels WHERE id = $1",
            )
            .bind(channel_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db_err)?
            .ok_or(BanterError::NotFound)?;
            let channel = channel_from_row(&row);
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
        "SELECT m.id, COALESCE(m.author_id, 0) AS author_id,
                COALESCE(u.username, m.plugin_id, '') AS username,
                COALESCE(u.display_name, m.plugin_id, '') AS display_name,
                mem.nickname, m.body, m.reply_to, m.created_at, m.edited_at,
                m.author_kind, m.plugin_id
         FROM messages m
         LEFT JOIN users u ON u.id = m.author_id
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
    let media = load_message_media(pool, message_id).await?;
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
        media,
        author_kind: row.get("author_kind"),
        plugin_id: row.get("plugin_id"),
    })
}

async fn load_message_tx(tx: &mut Transaction<'_, Postgres>, message_id: i64) -> Result<Message, BanterError> {
    let row = sqlx::query(
        "SELECT m.id, COALESCE(m.author_id, 0) AS author_id,
                COALESCE(u.username, m.plugin_id, '') AS username,
                COALESCE(u.display_name, m.plugin_id, '') AS display_name,
                mem.nickname, m.body, m.reply_to, m.created_at, m.edited_at,
                m.author_kind, m.plugin_id
         FROM messages m
         LEFT JOIN users u ON u.id = m.author_id
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
    let media = load_message_media_tx(tx, message_id).await?;
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
        media,
        author_kind: row.get("author_kind"),
        plugin_id: row.get("plugin_id"),
    })
}

async fn load_message_media(pool: &PgPool, message_id: i64) -> Result<Vec<MessageMedia>, BanterError> {
    let rows = sqlx::query(
        "SELECT mm.id, mm.asset_id, mm.kind, a.name, a.content_type, a.encoding,
                a.width, a.height, a.duration_ms
         FROM message_media mm
         JOIN emote_pack_assets a ON a.id = mm.asset_id
         WHERE mm.message_id = $1
         ORDER BY mm.sort_order, mm.id",
    )
    .bind(message_id)
    .fetch_all(pool)
    .await
    .map_err(db_err)?;
    rows_to_message_media(rows)
}

async fn load_message_media_tx(
    tx: &mut Transaction<'_, Postgres>,
    message_id: i64,
) -> Result<Vec<MessageMedia>, BanterError> {
    let rows = sqlx::query(
        "SELECT mm.id, mm.asset_id, mm.kind, a.name, a.content_type, a.encoding,
                a.width, a.height, a.duration_ms
         FROM message_media mm
         JOIN emote_pack_assets a ON a.id = mm.asset_id
         WHERE mm.message_id = $1
         ORDER BY mm.sort_order, mm.id",
    )
    .bind(message_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(db_err)?;
    rows_to_message_media(rows)
}

fn rows_to_message_media(rows: Vec<sqlx::postgres::PgRow>) -> Result<Vec<MessageMedia>, BanterError> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let kind = EmoteAssetKind::parse(row.get::<String, _>("kind").as_str())?;
        let encoding = row
            .get::<Option<String>, _>("encoding")
            .map(|v| EmoteGifEncoding::parse(&v))
            .transpose()?;
        out.push(MessageMedia {
            id: row.get("id"),
            asset_id: row.get("asset_id"),
            kind,
            name: row.get("name"),
            content_type: row.get("content_type"),
            encoding,
            width: row.get("width"),
            height: row.get("height"),
            duration_ms: row.get("duration_ms"),
        });
    }
    Ok(out)
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

pub(crate) fn normalize_username(raw: &str) -> Result<String, BanterError> {
    let name = raw.trim().to_lowercase();
    if username_ok(&name) {
        Ok(name)
    } else {
        Err(BanterError::InvalidUsername)
    }
}

pub(crate) fn clean_display(raw: &str) -> Result<String, BanterError> {
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

pub(crate) fn random_token() -> String {
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

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

pub(crate) fn db_err(err: sqlx::Error) -> BanterError {
    BanterError::Storage(err.to_string())
}

pub(crate) fn constraint(err: sqlx::Error, unique: BanterError) -> BanterError {
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
