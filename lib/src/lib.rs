#![forbid(unsafe_code)]
pub mod discord;
pub mod error;
pub mod flow;
pub mod meetup;
pub mod redis;
pub mod strings;
pub mod stripe;
pub mod tasks;
pub mod urls;

use ::redis::AsyncCommands;
use ::redis::Commands;
pub use error::BoxedError;
use rand::Rng;

pub type BoxedFuture<T> = Box<dyn std::future::Future<Output = T> + Send>;

pub fn new_random_id(num_bytes: u32) -> String {
    let random_bytes: Vec<u8> = (0..num_bytes)
        .map(|_| rand::thread_rng().gen::<u8>())
        .collect();
    base64::encode_config(&random_bytes, base64::URL_SAFE_NO_PAD)
}

pub struct ChannelRoles {
    pub user: u64,
    pub host: u64,
}

pub fn get_channel_roles(
    channel_id: u64,
    redis_connection: &mut ::redis::Connection,
) -> Result<Option<ChannelRoles>, crate::meetup::Error> {
    // Figure out whether this is a game channel
    let is_game_channel: bool = redis_connection.sismember("discord_channels", channel_id)?;
    if !is_game_channel {
        return Ok(None);
    }
    // Check that this message came from a bot controlled channel
    let redis_channel_role_key = format!("discord_channel:{}:discord_role", channel_id);
    let redis_channel_host_role_key = format!("discord_channel:{}:discord_host_role", channel_id);
    let channel_roles: ::redis::RedisResult<(Option<u64>, Option<u64>)> = ::redis::pipe()
        .get(redis_channel_role_key)
        .get(redis_channel_host_role_key)
        .query(redis_connection);
    match channel_roles {
        Ok((Some(role), Some(host_role))) => Ok(Some(ChannelRoles {
            user: role,
            host: host_role,
        })),
        Ok((None, None)) => Ok(None),
        Ok(_) => {
            return Err(simple_error::SimpleError::new("Channel has only one of two roles").into());
        }
        Err(err) => {
            return Err(err.into());
        }
    }
}

pub async fn get_channel_roles_async(
    channel_id: u64,
    redis_connection: &mut ::redis::aio::Connection,
) -> Result<Option<ChannelRoles>, crate::meetup::Error> {
    // Figure out whether this is a game channel
    let is_game_channel: bool = redis_connection
        .sismember("discord_channels", channel_id)
        .await?;
    if !is_game_channel {
        return Ok(None);
    }
    // Check that this message came from a bot controlled channel
    let redis_channel_role_key = format!("discord_channel:{}:discord_role", channel_id);
    let redis_channel_host_role_key = format!("discord_channel:{}:discord_host_role", channel_id);
    let channel_roles: ::redis::RedisResult<(Option<u64>, Option<u64>)> = ::redis::pipe()
        .get(redis_channel_role_key)
        .get(redis_channel_host_role_key)
        .query_async(redis_connection)
        .await;
    match channel_roles {
        Ok((Some(role), Some(host_role))) => Ok(Some(ChannelRoles {
            user: role,
            host: host_role,
        })),
        Ok((None, None)) => Ok(None),
        Ok(_) => {
            return Err(simple_error::SimpleError::new("Channel has only one of two roles").into());
        }
        Err(err) => {
            return Err(err.into());
        }
    }
}

pub fn get_event_series_roles(
    event_series_id: &str,
    redis_connection: &mut ::redis::Connection,
) -> Result<Option<ChannelRoles>, crate::meetup::Error> {
    let discord_channel: Option<u64> =
        redis_connection.get(format!("event_series:{}:discord_channel", event_series_id))?;
    if let Some(discord_channel) = discord_channel {
        get_channel_roles(discord_channel, redis_connection)
    } else {
        Ok(None)
    }
}

pub async fn get_event_series_roles_async(
    event_series_id: &str,
    redis_connection: &mut ::redis::aio::Connection,
) -> Result<Option<ChannelRoles>, crate::meetup::Error> {
    let discord_channel: Option<u64> = redis_connection
        .get(format!("event_series:{}:discord_channel", event_series_id))
        .await?;
    if let Some(discord_channel) = discord_channel {
        get_channel_roles_async(discord_channel, redis_connection).await
    } else {
        Ok(None)
    }
}
