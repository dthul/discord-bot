use redis::AsyncCommands;
use serenity::{builder::EditChannel, model::channel::Channel};
use std::time::Duration;

use crate::discord::sync::ids::GUILD_ID;

pub const DEFAULT_USER_TOPIC_VOICE_CHANNEL_NAME: &str = "Your topic (ask Hyperion)";

// Resets the user topic voice channel
pub async fn reset_user_topic_voice_channel_task(
    redis_client: redis::Client,
    mut discord_api: crate::discord::CacheAndHttp,
) -> ! {
    // Do this every minute
    let mut interval_timer = tokio::time::interval_at(
        tokio::time::Instant::now() + tokio::time::Duration::from_secs(10),
        tokio::time::Duration::from_secs(60),
    );
    // Run forever
    loop {
        // Wait for the next interval tick
        interval_timer.tick().await;
        let mut redis_connection = match redis_client.get_multiplexed_async_connection().await {
            Ok(con) => con,
            Err(err) => {
                eprintln!(
                    "User topic voice channel reset task: Could not acquire Redis connection:\n{:#?}",
                    err
                );
                continue;
            }
        };
        if let Err(err) =
            reset_user_topic_voice_channel(&mut redis_connection, &mut discord_api).await
        {
            eprintln!("User topic voice channel reset task failed:\n{:#?}", err);
        }
    }
}

async fn reset_user_topic_voice_channel(
    redis_connection: &mut redis::aio::MultiplexedConnection,
    discord_api: &crate::discord::CacheAndHttp,
) -> Result<(), crate::meetup::Error> {
    // Check if there is a user topic voice channel
    let voice_channel_id = if let Some(id) = crate::discord::sync::ids::USER_TOPIC_VOICE_CHANNEL_ID
    {
        id
    } else {
        return Ok(());
    };
    let mut voice_channel = voice_channel_id
        .to_guild_channel(discord_api, Some(GUILD_ID))
        .await?;
    // Check if the voice channel's name is different from the default
    if voice_channel.base.name == DEFAULT_USER_TOPIC_VOICE_CHANNEL_NAME {
        return Ok(());
    }
    // Check if the voice channel is empty
    if !voice_channel.members(&discord_api.cache)?.is_empty() {
        return Ok(());
    }
    // Check if the voice channel has not been renamed very recently
    let topic_time = match redis_connection
        .get::<_, Option<String>>("user_topic_voice_channel_topic_time")
        .await?
    {
        Some(time) => match chrono::DateTime::parse_from_rfc3339(&time) {
            Ok(time) => Some(time.with_timezone(&chrono::Utc)),
            Err(_) => None,
        },
        None => None,
    };
    if let Some(topic_time) = topic_time {
        if chrono::Utc::now() - topic_time < chrono::Duration::minutes(2) {
            return Ok(());
        }
    }
    // The channel's name is different from the default, the channel is empty
    // and it has not been renamed very recently, so we'll go ahead and reset
    // it.
    // Channel renaming is strongly ratelimited by Discord (at the time of this
    // writing it's twice every 10 minutes), so the following call can easily
    // block for a long time (because Serenity will not return an error when
    // rate limited but queue the call instead and execute it when possible).
    // We don't want that, we want to fail fast. That's why we wrap the renaming
    // call into a timeout which will abort the request if it is not answered
    // quickly enough.
    let rename_channel_future = voice_channel.edit(
        &discord_api.http,
        EditChannel::new().name(DEFAULT_USER_TOPIC_VOICE_CHANNEL_NAME),
    );
    match tokio::time::timeout(Duration::from_secs(10), rename_channel_future).await {
        Err(_) => eprintln!("User topic voice channel reset: channel edit request timed out"),
        Ok(e @ Err(_)) => e?,
        _ => (),
    }
    Ok(())
}
