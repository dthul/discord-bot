use chrono::DateTime;
use serenity::all::ChannelId;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum EndAdventureResult {
    NotAGameChannel,
    NoExpirationTime,
    NotYetExpired,
    AlreadyMarkedForDeletion(DateTime<chrono::Utc>),
    NewlyMarkedForDeletion(DateTime<chrono::Utc>),
}

pub async fn end_adventure(
    channel_id: ChannelId,
    db_connection: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<EndAdventureResult, crate::meetup::Error> {
    // Check whether this is a game channel
    let is_game_channel = crate::is_game_channel(channel_id, db_connection).await?;
    if !is_game_channel {
        return Ok(EndAdventureResult::NotAGameChannel);
    };
    // // Check if there is a channel expiration time in the future
    // let expiration_time = sqlx::query_scalar!(
    //     r#"SELECT expiration_time FROM event_series_text_channel WHERE discord_id = $1"#,
    //     channel_id.get() as i64
    // )
    // .fetch_one(&mut **db_connection)
    // .await?;
    // let expiration_time = if let Some(expiration_time) = expiration_time {
    //     expiration_time
    // } else {
    //     return Ok(EndAdventureResult::NoExpirationTime);
    // };
    // if expiration_time > chrono::Utc::now() {
    //     return Ok(EndAdventureResult::NotYetExpired);
    // }
    let expiration_time = chrono::Utc::now();
    // Schedule this channel for deletion
    let new_deletion_time = chrono::Utc::now() + chrono::Duration::hours(8);
    let current_deletion_time = sqlx::query_scalar!(
        r#"SELECT deletion_time FROM event_series_text_channel WHERE discord_id = $1"#,
        channel_id.get() as i64
    )
    .fetch_one(&mut **db_connection)
    .await?;
    if let Some(current_deletion_time) = current_deletion_time {
        if new_deletion_time > current_deletion_time && current_deletion_time > expiration_time {
            return Ok(EndAdventureResult::AlreadyMarkedForDeletion(
                current_deletion_time,
            ));
        }
    }
    // Figure out whether there is an associated voice channel
    let voice_channel_id = crate::get_channel_voice_channel(channel_id, db_connection).await?;
    let channel_roles = crate::get_channel_roles(channel_id, db_connection).await?;
    sqlx::query!(
        r#"UPDATE event_series_text_channel SET deletion_time = $2 WHERE discord_id = $1"#,
        channel_id.get() as i64,
        new_deletion_time
    )
    .execute(&mut **db_connection)
    .await?;
    // If there is an associated voice channel, mark it also for deletion
    if let Some(voice_channel_id) = voice_channel_id {
        sqlx::query!(
            r#"UPDATE event_series_voice_channel SET deletion_time = $2 WHERE discord_id = $1"#,
            voice_channel_id.get() as i64,
            new_deletion_time
        )
        .execute(&mut **db_connection)
        .await?;
    }
    if let Some(channel_roles) = channel_roles {
        sqlx::query!(
            r#"UPDATE event_series_role SET deletion_time = $2 WHERE discord_id = $1"#,
            channel_roles.user.get() as i64,
            new_deletion_time
        )
        .execute(&mut **db_connection)
        .await?;
        if let Some(host_role_id) = channel_roles.host {
            sqlx::query!(
                r#"UPDATE event_series_host_role SET deletion_time = $2 WHERE discord_id = $1"#,
                host_role_id.get() as i64,
                new_deletion_time
            )
            .execute(&mut **db_connection)
            .await?;
        }
    }
    Ok(EndAdventureResult::NewlyMarkedForDeletion(
        new_deletion_time,
    ))
}
