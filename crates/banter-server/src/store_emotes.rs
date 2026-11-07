//! Emote Library store methods (BA-83).

use std::fs;

use sqlx::Row;

use crate::emotes::{
    classic_gif_duration_ms, max_bytes_for_kind, video_container_duration_ms, GIF_MAX_DURATION_MS,
};
use crate::error::BanterError;
use crate::model::*;
use crate::store::{db_err, now_ms, Store};

impl Store {
    pub async fn create_emote_pack(
        &self,
        user_id: i64,
        name: &str,
        slug: &str,
        description: &str,
        visibility: EmotePackVisibility,
        owner: EmotePackOwner,
    ) -> Result<EmotePack, BanterError> {
        let name = clean_pack_name(name)?;
        let slug = clean_pack_slug(slug)?;
        let description = description.trim();
        if description.len() > 500 {
            return Err(BanterError::InvalidName);
        }
        let (owner_kind, owner_id) = match owner {
            EmotePackOwner::User => ("user", user_id),
            EmotePackOwner::Server(server_id) => {
                self.require_server_owner(user_id, server_id).await?;
                ("server", server_id)
            }
        };
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO emote_packs
                (name, slug, description, cover_asset_id, owner_kind, owner_id, visibility, created_by, created_at)
             VALUES ($1, $2, $3, NULL, $4, $5, $6, $7, $8)
             RETURNING id",
        )
        .bind(&name)
        .bind(&slug)
        .bind(description)
        .bind(owner_kind)
        .bind(owner_id)
        .bind(visibility.as_str())
        .bind(user_id)
        .bind(now_ms())
        .fetch_one(&self.pool)
        .await
        .map_err(|err| {
            if let sqlx::Error::Database(db) = &err {
                if db.constraint().is_some_and(|c| c.contains("slug")) {
                    return BanterError::Conflict;
                }
            }
            db_err(err)
        })?;
        if let EmotePackOwner::Server(server_id) = owner {
            sqlx::query(
                "INSERT INTO server_emote_packs (server_id, pack_id) VALUES ($1, $2)
                 ON CONFLICT DO NOTHING",
            )
            .bind(server_id)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        }
        self.load_emote_pack(id).await
    }

    pub async fn upload_emote_asset(
        &self,
        user_id: i64,
        pack_id: i64,
        name: &str,
        kind: EmoteAssetKind,
        content_type: &str,
        bytes: &[u8],
    ) -> Result<EmoteAsset, BanterError> {
        let pack = self.load_emote_pack(pack_id).await?;
        self.require_pack_editor(user_id, &pack).await?;
        let name = clean_asset_name(name)?;
        let content_type = content_type.trim().to_ascii_lowercase();
        let (encoding, duration_ms, width, height) =
            inspect_upload(kind, &content_type, bytes)?;
        let max = max_bytes_for_kind(kind, encoding);
        if bytes.is_empty() || bytes.len() > max {
            return Err(BanterError::FileTooLarge);
        }
        if let Some(ms) = duration_ms {
            if ms > GIF_MAX_DURATION_MS {
                return Err(BanterError::EmoteDurationTooLong);
            }
        }
        let sort_order: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM emote_pack_assets WHERE pack_id = $1",
        )
        .bind(pack_id)
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO emote_pack_assets
                (pack_id, kind, name, encoding, content_type, byte_size, width, height, duration_ms, sort_order, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
             RETURNING id",
        )
        .bind(pack_id)
        .bind(kind.as_str())
        .bind(&name)
        .bind(encoding.map(|e| e.as_str()))
        .bind(&content_type)
        .bind(bytes.len() as i64)
        .bind(width)
        .bind(height)
        .bind(duration_ms)
        .bind(sort_order)
        .bind(now_ms())
        .fetch_one(&self.pool)
        .await
        .map_err(|err| {
            if let sqlx::Error::Database(db) = &err {
                if db.constraint().is_some_and(|c| c.contains("name")) {
                    return BanterError::Conflict;
                }
            }
            db_err(err)
        })?;
        let path = self.root.join("emotes").join(id.to_string());
        fs::write(&path, bytes).map_err(|err| BanterError::Storage(err.to_string()))?;
        self.load_emote_asset(id).await
    }

    pub async fn read_emote_asset(
        &self,
        user_id: i64,
        asset_id: i64,
    ) -> Result<(EmoteAsset, Vec<u8>), BanterError> {
        let asset = self.load_emote_asset(asset_id).await?;
        let pack = self.load_emote_pack(asset.pack_id).await?;
        self.require_pack_readable(user_id, &pack).await?;
        let bytes = fs::read(self.root.join("emotes").join(asset.id.to_string()))
            .map_err(|err| BanterError::Storage(err.to_string()))?;
        Ok((asset, bytes))
    }

    pub async fn subscribe_emote_pack(&self, user_id: i64, pack_id: i64) -> Result<(), BanterError> {
        let pack = self.load_emote_pack(pack_id).await?;
        self.require_pack_readable(user_id, &pack).await?;
        sqlx::query(
            "INSERT INTO emote_pack_subscriptions (user_id, pack_id, created_at)
             VALUES ($1, $2, $3)
             ON CONFLICT DO NOTHING",
        )
        .bind(user_id)
        .bind(pack_id)
        .bind(now_ms())
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    pub async fn unsubscribe_emote_pack(&self, user_id: i64, pack_id: i64) -> Result<(), BanterError> {
        sqlx::query(
            "DELETE FROM emote_pack_subscriptions WHERE user_id = $1 AND pack_id = $2",
        )
        .bind(user_id)
        .bind(pack_id)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    pub async fn list_emote_assets_for_picker(
        &self,
        user_id: i64,
        server_id: Option<i64>,
        kind: EmoteAssetKind,
        source: EmotePickerSource,
    ) -> Result<Vec<EmoteAsset>, BanterError> {
        match source {
            EmotePickerSource::Server => {
                let Some(server_id) = server_id else {
                    return Ok(Vec::new());
                };
                self.require_member(user_id, server_id).await?;
                let rows = sqlx::query(
                    "SELECT a.id, a.pack_id, a.kind, a.name, a.encoding, a.content_type, a.byte_size,
                            a.width, a.height, a.duration_ms, a.sort_order, a.created_at
                     FROM emote_pack_assets a
                     JOIN emote_packs p ON p.id = a.pack_id
                     WHERE a.kind = $1
                       AND (
                         (p.owner_kind = 'server' AND p.owner_id = $2)
                         OR EXISTS (
                             SELECT 1 FROM server_emote_packs s
                             WHERE s.server_id = $2 AND s.pack_id = p.id
                         )
                       )
                     ORDER BY a.sort_order, a.id",
                )
                .bind(kind.as_str())
                .bind(server_id)
                .fetch_all(&self.pool)
                .await
                .map_err(db_err)?;
                rows_to_assets(rows)
            }
            EmotePickerSource::Subscribed => {
                let rows = sqlx::query(
                    "SELECT a.id, a.pack_id, a.kind, a.name, a.encoding, a.content_type, a.byte_size,
                            a.width, a.height, a.duration_ms, a.sort_order, a.created_at
                     FROM emote_pack_assets a
                     JOIN emote_pack_subscriptions s ON s.pack_id = a.pack_id
                     WHERE s.user_id = $1 AND a.kind = $2
                     ORDER BY a.sort_order, a.id",
                )
                .bind(user_id)
                .bind(kind.as_str())
                .fetch_all(&self.pool)
                .await
                .map_err(db_err)?;
                rows_to_assets(rows)
            }
        }
    }

    pub(crate) async fn resolve_send_media(
        &self,
        user_id: i64,
        target: ChatTarget,
        media_asset_ids: &[i64],
    ) -> Result<Vec<(i64, EmoteAssetKind)>, BanterError> {
        if media_asset_ids.is_empty() {
            return Ok(Vec::new());
        }
        let server_id = match target {
            ChatTarget::Channel(channel_id) => {
                let channel = crate::store::channel_by_id(&self.pool, channel_id).await?;
                Some(channel.server_id)
            }
            ChatTarget::Direct(_) => None,
        };
        let mut out = Vec::with_capacity(media_asset_ids.len());
        let mut sticker_count = 0usize;
        let mut gif_count = 0usize;
        for &asset_id in media_asset_ids {
            let asset = self.load_emote_asset(asset_id).await?;
            if asset.kind == EmoteAssetKind::Emote {
                return Err(BanterError::Invalid);
            }
            let pack = self.load_emote_pack(asset.pack_id).await?;
            if !self.user_can_use_pack(user_id, server_id, &pack).await? {
                return Err(BanterError::NotMember);
            }
            match asset.kind {
                EmoteAssetKind::Sticker => sticker_count += 1,
                EmoteAssetKind::Gif => gif_count += 1,
                EmoteAssetKind::Emote => {}
            }
            out.push((asset.id, asset.kind));
        }
        // Model A: at most one sticker; do not mix stickers with GIFs in one send.
        if sticker_count > 1 || (sticker_count > 0 && gif_count > 0) {
            return Err(BanterError::Invalid);
        }
        Ok(out)
    }

    async fn load_emote_pack(&self, pack_id: i64) -> Result<EmotePack, BanterError> {
        let row = sqlx::query(
            "SELECT id, name, slug, description, cover_asset_id, owner_kind, owner_id,
                    visibility, created_by, created_at
             FROM emote_packs WHERE id = $1",
        )
        .bind(pack_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?
        .ok_or(BanterError::NotFound)?;
        Ok(EmotePack {
            id: row.get("id"),
            name: row.get("name"),
            slug: row.get("slug"),
            description: row.get("description"),
            cover_asset_id: row.get("cover_asset_id"),
            owner_kind: row.get("owner_kind"),
            owner_id: row.get("owner_id"),
            visibility: row.get("visibility"),
            created_by: row.get("created_by"),
            created_at: row.get("created_at"),
        })
    }

    async fn load_emote_asset(&self, asset_id: i64) -> Result<EmoteAsset, BanterError> {
        let row = sqlx::query(
            "SELECT id, pack_id, kind, name, encoding, content_type, byte_size,
                    width, height, duration_ms, sort_order, created_at
             FROM emote_pack_assets WHERE id = $1",
        )
        .bind(asset_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?
        .ok_or(BanterError::NotFound)?;
        asset_from_row(&row)
    }

    async fn require_server_owner(&self, user_id: i64, server_id: i64) -> Result<(), BanterError> {
        let owner: Option<i64> =
            sqlx::query_scalar("SELECT owner_id FROM servers WHERE id = $1")
                .bind(server_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(db_err)?;
        match owner {
            Some(owner) if owner == user_id => Ok(()),
            Some(_) => Err(BanterError::NotOwner),
            None => Err(BanterError::NotFound),
        }
    }

    async fn require_pack_editor(&self, user_id: i64, pack: &EmotePack) -> Result<(), BanterError> {
        match pack.owner_kind.as_str() {
            "user" if pack.owner_id == user_id => Ok(()),
            "server" => self.require_server_owner(user_id, pack.owner_id).await,
            _ => Err(BanterError::NotOwner),
        }
    }

    async fn require_pack_readable(&self, user_id: i64, pack: &EmotePack) -> Result<(), BanterError> {
        if self.user_can_use_pack(user_id, None, pack).await? {
            return Ok(());
        }
        // Public packs are discoverable; private needs ownership/membership.
        if pack.visibility == "public" || pack.visibility == "unlisted" {
            if pack.owner_kind == "user" && pack.owner_id == user_id {
                return Ok(());
            }
            if pack.owner_kind == "server" {
                if self.require_member(user_id, pack.owner_id).await.is_ok() {
                    return Ok(());
                }
            }
            if pack.visibility == "public" {
                return Ok(());
            }
        }
        if pack.owner_kind == "user" && pack.owner_id == user_id {
            return Ok(());
        }
        Err(BanterError::NotMember)
    }

    async fn user_can_use_pack(
        &self,
        user_id: i64,
        server_id: Option<i64>,
        pack: &EmotePack,
    ) -> Result<bool, BanterError> {
        if pack.owner_kind == "user" && pack.owner_id == user_id {
            return Ok(true);
        }
        let subscribed: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM emote_pack_subscriptions WHERE user_id = $1 AND pack_id = $2",
        )
        .bind(user_id)
        .bind(pack.id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        if subscribed.is_some() {
            return Ok(true);
        }
        if let Some(server_id) = server_id {
            if pack.owner_kind == "server" && pack.owner_id == server_id {
                self.require_member(user_id, server_id).await?;
                return Ok(true);
            }
            let linked: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM server_emote_packs WHERE server_id = $1 AND pack_id = $2",
            )
            .bind(server_id)
            .bind(pack.id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
            if linked.is_some() {
                self.require_member(user_id, server_id).await?;
                return Ok(true);
            }
        }
        Ok(false)
    }
}

fn clean_pack_name(name: &str) -> Result<String, BanterError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(BanterError::InvalidName);
    }
    Ok(name.to_string())
}

fn clean_pack_slug(slug: &str) -> Result<String, BanterError> {
    let slug = slug.trim().to_ascii_lowercase();
    if slug.len() < 2 || slug.len() > 64 {
        return Err(BanterError::InvalidName);
    }
    if !slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(BanterError::InvalidName);
    }
    Ok(slug)
}

fn clean_asset_name(name: &str) -> Result<String, BanterError> {
    let name = name.trim();
    if name.is_empty() || name.len() > 64 {
        return Err(BanterError::InvalidName);
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(BanterError::InvalidName);
    }
    Ok(name.to_string())
}

fn inspect_upload(
    kind: EmoteAssetKind,
    content_type: &str,
    bytes: &[u8],
) -> Result<(Option<EmoteGifEncoding>, Option<i64>, Option<i64>, Option<i64>), BanterError> {
    match kind {
        EmoteAssetKind::Emote | EmoteAssetKind::Sticker => {
            if !matches!(
                content_type,
                "image/png" | "image/jpeg" | "image/jpg" | "image/webp" | "image/gif"
            ) {
                return Err(BanterError::Invalid);
            }
            let (w, h) = image_dims(bytes).unwrap_or((None, None));
            let duration = if content_type == "image/gif" {
                classic_gif_duration_ms(bytes).ok()
            } else {
                None
            };
            if let Some(ms) = duration {
                if ms > GIF_MAX_DURATION_MS {
                    return Err(BanterError::EmoteDurationTooLong);
                }
            }
            Ok((None, duration, w, h))
        }
        EmoteAssetKind::Gif => {
            if content_type == "image/gif" {
                let duration = classic_gif_duration_ms(bytes).map_err(|_| BanterError::Invalid)?;
                let (w, h) = image_dims(bytes).unwrap_or((None, None));
                Ok((Some(EmoteGifEncoding::ClassicGif), Some(duration), w, h))
            } else if content_type == "video/mp4" || content_type == "video/webm" {
                let duration =
                    video_container_duration_ms(bytes).map_err(|_| BanterError::Invalid)?;
                Ok((Some(EmoteGifEncoding::Video), Some(duration), None, None))
            } else {
                Err(BanterError::Invalid)
            }
        }
    }
}

fn image_dims(bytes: &[u8]) -> Result<(Option<i64>, Option<i64>), ()> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| ())?;
    let (w, h) = reader.into_dimensions().map_err(|_| ())?;
    Ok((Some(w as i64), Some(h as i64)))
}

fn rows_to_assets(rows: Vec<sqlx::postgres::PgRow>) -> Result<Vec<EmoteAsset>, BanterError> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(asset_from_row(&row)?);
    }
    Ok(out)
}

fn asset_from_row(row: &sqlx::postgres::PgRow) -> Result<EmoteAsset, BanterError> {
    let kind = EmoteAssetKind::parse(row.get::<String, _>("kind").as_str())?;
    let encoding = row
        .get::<Option<String>, _>("encoding")
        .map(|v| EmoteGifEncoding::parse(&v))
        .transpose()?;
    Ok(EmoteAsset {
        id: row.get("id"),
        pack_id: row.get("pack_id"),
        kind,
        name: row.get("name"),
        encoding,
        content_type: row.get("content_type"),
        byte_size: row.get("byte_size"),
        width: row.get("width"),
        height: row.get("height"),
        duration_ms: row.get("duration_ms"),
        sort_order: row.get("sort_order"),
        created_at: row.get("created_at"),
    })
}
