pub mod sync;
pub mod util;

use std::sync::Arc;

use serenity::model::{
    channel::{Channel, PermissionOverwrite, PermissionOverwriteType},
    id::{ChannelId, UserId},
    permissions::Permissions,
};
use tracing::info;

#[derive(Clone)]
pub struct CacheAndHttp {
    pub cache: Arc<serenity::cache::Cache>,
    pub http: Arc<serenity::http::Http>,
}

impl serenity::http::CacheHttp for CacheAndHttp {
    fn cache(&self) -> Option<&Arc<serenity::cache::Cache>> {
        Some(&self.cache)
    }
    fn http(&self) -> &serenity::http::Http {
        &self.http
    }
}

impl From<&serenity::client::Context> for CacheAndHttp {
    fn from(ctx: &serenity::client::Context) -> Self {
        CacheAndHttp {
            cache: ctx.cache.clone(),
            http: ctx.http.clone(),
        }
    }
}

#[tracing::instrument(skip(discord_api, db_connection))]
pub async fn is_host(
    discord_api: &CacheAndHttp,
    channel_id: ChannelId,
    user_id: UserId,
    db_connection: &sqlx::PgPool,
) -> Result<bool, crate::meetup::Error> {
    let channel = if let Channel::Guild(channel) = channel_id.to_channel(discord_api).await? {
        channel
    } else {
        return Err(simple_error::SimpleError::new("is_host: This is not a guild channel").into());
    };
    // Assume that users with the VIEW_CHANNEL, MANAGE_MESSAGES and
    // MENTION_EVERYONE permission are channel hosts
    let user_permission_overwrites = channel
        .permission_overwrites
        .iter()
        .find(|overwrite| PermissionOverwriteType::Member(user_id) == overwrite.kind)
        .cloned();
    let is_host = user_permission_overwrites.map_or(false, |overwrites| {
        overwrites.allow.contains(
            Permissions::VIEW_CHANNEL
                | Permissions::MANAGE_MESSAGES
                | Permissions::MENTION_EVERYONE,
        )
    });
    if !is_host {
        // Maybe the user is still on the old host roles
        let channel_roles =
            crate::get_channel_roles(channel_id, &mut db_connection.begin().await?).await?;
        if let Some(crate::ChannelRoles {
            host: Some(host_role),
            ..
        }) = channel_roles
        {
            let user = user_id.to_user(discord_api).await?;
            let is_host = user
                .has_role(discord_api, sync::ids::GUILD_ID, host_role)
                .await
                .unwrap_or(false);
            return Ok(is_host);
        } else {
            return Ok(false);
        }
    }
    Ok(is_host)
}

// True if permissions changed, false otherwise
#[tracing::instrument(skip(discord_api))]
pub async fn add_channel_user_permissions(
    discord_api: &CacheAndHttp,
    channel_id: ChannelId,
    user_id: UserId,
    permissions: Permissions,
) -> Result<bool, crate::meetup::Error> {
    if permissions == Permissions::empty() {
        return Ok(false);
    }
    let channel = if let Channel::Guild(channel) = channel_id.to_channel(discord_api).await? {
        channel
    } else {
        return Err(simple_error::SimpleError::new(
            "add_channel_user_permissions: This is not a guild channel",
        )
        .into());
    };
    let current_permission_overwrites = channel
        .permission_overwrites
        .iter()
        .find(|overwrite| PermissionOverwriteType::Member(user_id) == overwrite.kind)
        .cloned()
        .unwrap_or(PermissionOverwrite {
            allow: Permissions::empty(),
            deny: Permissions::empty(),
            kind: PermissionOverwriteType::Member(user_id),
        });
    let mut new_permission_overwrites = current_permission_overwrites.clone();
    new_permission_overwrites.allow |= permissions;
    if new_permission_overwrites.allow != current_permission_overwrites.allow {
        info!(
            ?current_permission_overwrites,
            ?new_permission_overwrites,
            "Adding channel permissions",
        );
        channel
            .create_permission(&discord_api.http, new_permission_overwrites)
            .await?;
        Ok(true)
    } else {
        Ok(false)
    }
}

// True if permissions changed, false otherwise
#[tracing::instrument(skip(discord_api))]
pub async fn remove_channel_user_permissions(
    discord_api: &CacheAndHttp,
    channel_id: ChannelId,
    user_id: UserId,
    permissions: Permissions,
) -> Result<bool, crate::meetup::Error> {
    if permissions == Permissions::empty() {
        return Ok(false);
    }
    let channel = if let Channel::Guild(channel) = channel_id.to_channel(discord_api).await? {
        channel
    } else {
        return Err(simple_error::SimpleError::new(
            "remove_channel_user_permissions: This is not a guild channel",
        )
        .into());
    };
    let current_permission_overwrites = channel
        .permission_overwrites
        .iter()
        .find(|overwrite| PermissionOverwriteType::Member(user_id) == overwrite.kind)
        .cloned()
        .unwrap_or(PermissionOverwrite {
            allow: Permissions::empty(),
            deny: Permissions::empty(),
            kind: PermissionOverwriteType::Member(user_id),
        });
    let mut new_permission_overwrites = current_permission_overwrites.clone();
    new_permission_overwrites.allow &= !permissions;
    if new_permission_overwrites.allow != current_permission_overwrites.allow {
        info!(
            ?current_permission_overwrites,
            ?new_permission_overwrites,
            "Removing channel permissions",
        );
        if new_permission_overwrites.allow == Permissions::empty()
            && new_permission_overwrites.deny == Permissions::empty()
        {
            channel
                .delete_permission(&discord_api.http, new_permission_overwrites.kind)
                .await?;
        } else {
            channel
                .create_permission(&discord_api.http, new_permission_overwrites)
                .await?;
        }
        Ok(true)
    } else {
        Ok(false)
    }
}
