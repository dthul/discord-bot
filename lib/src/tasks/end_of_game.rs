use crate::{db, discord::sync::ChannelType, strings};
use chrono::{NaiveTime, TimeZone};
use serenity::{
    builder::CreateMessage,
    model::{
        channel::GuildChannel,
        guild::Role,
        id::{ChannelId, RoleId, UserId},
    },
};
use simple_error::SimpleError;

// Sends channel deletion reminders to expired Discord channels
pub async fn create_recurring_end_of_game_task(
    db_connection: sqlx::PgPool,
    mut discord_api: crate::discord::CacheAndHttp,
    bot_id: UserId,
) -> ! {
    let next_end_of_game_task_time = {
        let mut task_time = chrono::Utc
            .from_local_datetime(
                &chrono::Utc::now()
                    .date_naive()
                    .and_time(NaiveTime::from_hms_opt(18, 30, 0).unwrap()),
            )
            .earliest()
            .expect("18:30 o'clock exists today");
        // Check if it is later than 6:30pm
        // In that case, run the task tomorrow
        if chrono::Utc::now() > task_time {
            task_time = task_time + chrono::Duration::days(1);
        }
        task_time
    };
    let wait_duration = next_end_of_game_task_time - chrono::Utc::now();
    // Do this once a day, starting from the specified time
    let mut interval_timer = tokio::time::interval_at(
        tokio::time::Instant::now() + wait_duration.to_std().unwrap_or_default(),
        chrono::Duration::days(1).to_std().unwrap(),
    );
    // Run forever
    loop {
        // Wait for the next interval tick
        interval_timer.tick().await;
        if let Err(err) = end_of_game_task(&db_connection, &mut discord_api, bot_id).await {
            eprintln!("End of game task failed:\n{:#?}", err);
        }
    }
}

pub async fn end_of_game_task(
    db_connection: &sqlx::PgPool,
    discord_api: &mut crate::discord::CacheAndHttp,
    bot_id: UserId,
) -> Result<(), crate::meetup::Error> {
    let event_series = sqlx::query!(
        r#"SELECT event_series.id as "event_series_id!"
        FROM event_series
        INNER JOIN event_series_text_channel ON event_series.discord_text_channel_id = event_series_text_channel.discord_id
        WHERE event_series_text_channel.deleted IS NULL"#
    ).map(|row| db::EventSeriesId(row.event_series_id)).fetch_all(db_connection).await?;
    let mut some_failed = false;
    // First, update each series' channel's expiration time
    for series_id in event_series {
        if let Err(err) = update_series_channel_expiration(series_id, db_connection).await {
            some_failed = true;
            eprintln!("Series channel expiration update failed: {:#}", err);
        }
    }
    let existing_channels: Vec<GuildChannel> = crate::discord::sync::ids::GUILD_ID
        .channels(&discord_api.http)
        .await?
        .into_iter()
        .collect();
    let discord_channels = sqlx::query!(
        r#"
        SELECT discord_id as "discord_text_channel_id!"
        FROM event_series_text_channel
        WHERE deleted IS NULL"#
    )
    .map(|row| ChannelId::new(row.discord_text_channel_id as u64))
    .fetch_all(db_connection)
    .await?;
    for channel in discord_channels {
        // Then, check if the channel is due for deletion
        match delete_marked_channel(
            ChannelType::Text,
            channel,
            &existing_channels,
            db_connection,
            discord_api,
        )
        .await
        {
            Ok(deletion_status) => {
                if deletion_status == DeletionStatus::NotDeleted {
                    // Lastly, send a reminder if necessary
                    if let Err(err) = send_channel_expiration_reminder(
                        channel,
                        db_connection,
                        discord_api,
                        bot_id,
                    )
                    .await
                    {
                        some_failed = true;
                        eprintln!("Channel expiration reminder failed: {:#}", err);
                    }
                }
            }
            Err(err) => {
                some_failed = true;
                eprintln!("Error during channel deletion: {:#}", err);
            }
        }
    }
    let discord_voice_channels = sqlx::query!(
        r#"SELECT discord_id as "discord_voice_channel_id!"
        FROM event_series_voice_channel
        WHERE deleted IS NULL AND deletion_time IS NOT NULL"#
    )
    .map(|row| ChannelId::new(row.discord_voice_channel_id as u64))
    .fetch_all(db_connection)
    .await?;
    for channel in discord_voice_channels {
        // Check if the channel is due for deletion
        if let Err(err) = delete_marked_channel(
            ChannelType::Voice,
            channel,
            &existing_channels,
            db_connection,
            discord_api,
        )
        .await
        {
            some_failed = true;
            eprintln!("Error during voice channel deletion: {:#}", err);
        }
    }
    let existing_roles: Vec<Role> = crate::discord::sync::ids::GUILD_ID
        .roles(&discord_api.http)
        .await?
        .into_iter()
        .collect();
    let discord_roles = sqlx::query!(
        r#"SELECT discord_id as "discord_role_id!"
            FROM event_series_role
            WHERE deleted IS NULL AND deletion_time IS NOT NULL"#
    )
    .map(|row| RoleId::new(row.discord_role_id as u64))
    .fetch_all(db_connection)
    .await?;
    for role in discord_roles {
        // Check if the role is due for deletion
        if let Err(err) =
            delete_marked_role(false, role, &existing_roles, db_connection, discord_api).await
        {
            some_failed = true;
            eprintln!("Error during role deletion: {:#}", err);
        }
    }
    let discord_host_roles = sqlx::query!(
        r#"SELECT discord_id as "discord_role_id!"
            FROM event_series_host_role
            WHERE deleted IS NULL AND deletion_time IS NOT NULL"#
    )
    .map(|row| RoleId::new(row.discord_role_id as u64))
    .fetch_all(db_connection)
    .await?;
    for role in discord_host_roles {
        // Check if the host role is due for deletion
        if let Err(err) =
            delete_marked_role(true, role, &existing_roles, db_connection, discord_api).await
        {
            some_failed = true;
            eprintln!("Error during host role deletion: {:#}", err);
        }
    }
    if some_failed {
        Err(SimpleError::new("One or more end of game tasks failed").into())
    } else {
        Ok(())
    }
}

async fn update_series_channel_expiration(
    series_id: db::EventSeriesId,
    db_connection: &sqlx::PgPool,
) -> Result<(), crate::meetup::Error> {
    let res = sqlx::query!(r#"SELECT discord_text_channel_id, discord_voice_channel_id, discord_role_id, discord_host_role_id FROM event_series WHERE id = $1"#, series_id.0).fetch_one(db_connection).await?;
    let discord_text_channel_id = res
        .discord_text_channel_id
        .map(|id| ChannelId::new(id as u64));
    let discord_voice_channel_id = res
        .discord_voice_channel_id
        .map(|id| ChannelId::new(id as u64));
    let discord_role_id = res.discord_role_id.map(|id| RoleId::new(id as u64));
    let discord_host_role_id = res.discord_host_role_id.map(|id| RoleId::new(id as u64));
    let discord_text_channel_id = if let Some(id) = discord_text_channel_id {
        id
    } else {
        println!(
            "Expiration update: Event series {} has no channel associated with it",
            series_id.0
        );
        return Ok(());
    };
    // Get last event in this series
    let last_event = db::get_last_event_in_series(db_connection, series_id).await?;
    // Query the channel's current expiration time
    let current_expiration_time = sqlx::query_scalar!(
        r#"SELECT expiration_time FROM event_series_text_channel WHERE discord_id = $1"#,
        discord_text_channel_id.get() as i64
    )
    .fetch_one(db_connection)
    .await?;
    // The last element in this vector will be the last event in the series
    let (new_expiration_time, needs_update) = if let Some(last_event) = last_event {
        let new_expiration_time = last_event.time;
        let needs_update = current_expiration_time
            .map(|current| current != new_expiration_time)
            .unwrap_or(true);
        (new_expiration_time, needs_update)
    } else {
        // No event in this series, expire the channel immediately
        let new_expiration_time = chrono::Utc::now();
        let needs_update = current_expiration_time
            .map(|current| current > new_expiration_time)
            .unwrap_or(true);
        (new_expiration_time, needs_update)
    };
    // Store the new expiration time
    if needs_update {
        // Also delete any possibly stored deletion times from the channel, the
        // possibly associated voice channel and roles
        let mut tx = db_connection.begin().await?;
        sqlx::query!(
            r#"UPDATE event_series_text_channel SET expiration_time = $2, deletion_time = NULL WHERE discord_id = $1"#,
            discord_text_channel_id.get() as i64,
            new_expiration_time
        )
        .execute(&mut *tx)
        .await?;
        if let Some(discord_voice_channel_id) = discord_voice_channel_id {
            sqlx::query!(
                r#"UPDATE event_series_voice_channel SET deletion_time = NULL WHERE discord_id = $1"#,
                discord_voice_channel_id.get() as i64
            )
            .execute(&mut *tx)
            .await?;
        }
        if let Some(discord_role_id) = discord_role_id {
            sqlx::query!(
                r#"UPDATE event_series_role SET deletion_time = NULL WHERE discord_id = $1"#,
                discord_role_id.get() as i64
            )
            .execute(&mut *tx)
            .await?;
        }
        if let Some(discord_host_role_id) = discord_host_role_id {
            sqlx::query!(
                r#"UPDATE event_series_host_role SET deletion_time = NULL WHERE discord_id = $1"#,
                discord_host_role_id.get() as i64
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        println!(
            "Set expiration time of channel {} to {}",
            discord_text_channel_id, new_expiration_time
        );
    }
    Ok(())
}

async fn send_channel_expiration_reminder(
    channel_id: ChannelId,
    db_connection: &sqlx::PgPool,
    discord_api: &mut crate::discord::CacheAndHttp,
    bot_id: UserId,
) -> Result<(), crate::meetup::Error> {
    let (expiration_time, last_reminder_time, snooze_until, deletion_time) = sqlx::query!(
        r#"SELECT expiration_time, last_expiration_reminder_time, snooze_until, deletion_time
        FROM event_series_text_channel
        WHERE discord_id = $1"#,
        channel_id.get() as i64
    )
    .map(|row| {
        (
            row.expiration_time,
            row.last_expiration_reminder_time,
            row.snooze_until,
            row.deletion_time,
        )
    })
    .fetch_one(db_connection)
    .await?;
    if deletion_time.is_some() {
        // This channel is already marked for deletion, don't send another reminder
        return Ok(());
    }
    if let Some(expiration_time) = expiration_time {
        // Check if this is a one-shot or a campaign series
        let is_campaign = sqlx::query_scalar!(
            r#"SELECT "type" = 'campaign' as "is_campaign!" FROM event_series WHERE discord_text_channel_id = $1"#,
            channel_id.get() as i64
        )
        .fetch_one(db_connection)
        .await?;
        // We only remind a certain time after expiration
        let reminder_time = if is_campaign {
            expiration_time + chrono::Duration::days(3)
        } else {
            expiration_time + chrono::Duration::days(1)
        };
        let now = chrono::Utc::now();
        if reminder_time > now {
            // The reminder time hasn't come yet
            return Ok(());
        }
        if let Some(snooze_until) = snooze_until {
            if snooze_until > now {
                // Reminders are snoozed
                return Ok(());
            }
        }
        if let Some(last_reminder_time) = last_reminder_time {
            // Reminders will be sent with an interval of two days for
            // one-shots and four days for campaign channels
            let reminder_interval = if is_campaign {
                chrono::Duration::days(4) - chrono::Duration::hours(2)
            } else {
                chrono::Duration::days(2) - chrono::Duration::hours(2)
            };
            if last_reminder_time + reminder_interval > chrono::Utc::now() {
                // We already sent a reminder recently
                return Ok(());
            }
        }
        // Send a reminder and update the last reminder time
        println!("Reminding channel {} of its expiration", channel_id);
        let channel_roles =
            crate::get_channel_roles(channel_id, &mut db_connection.begin().await?).await?;
        let user_role = channel_roles.map(|roles| roles.user);
        let message_builder = CreateMessage::new().content(if is_campaign {
            strings::END_OF_CAMPAIGN_MESSAGE(bot_id, user_role)
        } else {
            strings::END_OF_ADVENTURE_MESSAGE(bot_id, user_role)
        });
        channel_id
            .widen()
            .send_message(&discord_api.http, message_builder)
            .await?;
        sqlx::query!("UPDATE event_series_text_channel SET last_expiration_reminder_time = NOW() WHERE discord_id = $1", channel_id.get() as i64).execute(db_connection).await?;
        println!(
            "Updated channel's {} latest expiration reminder time",
            channel_id
        );
    }
    Ok(())
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum DeletionStatus {
    Deleted,
    NotDeleted,
    AlreadyDeleted,
}

async fn delete_marked_channel(
    channel_type: ChannelType,
    channel_id: ChannelId,
    existing_channels: &[GuildChannel],
    db_connection: &sqlx::PgPool,
    discord_api: &crate::discord::CacheAndHttp,
) -> Result<DeletionStatus, crate::meetup::Error> {
    let mark_channel_as_deleted = || async {
        match channel_type {
            ChannelType::Text => {
                sqlx::query!("UPDATE event_series_text_channel SET deleted = NOW() WHERE discord_id = $1 AND (deleted IS NULL OR deleted > NOW())", channel_id.get() as i64).execute(db_connection).await
            },
            ChannelType::Voice => {
                sqlx::query!("UPDATE event_series_voice_channel SET deleted = NOW() WHERE discord_id = $1 AND (deleted IS NULL OR deleted > NOW())", channel_id.get() as i64).execute(db_connection).await
            }
        }
    };
    // Check whether the channel still exists on Discord
    let channel = if let Some(channel) = existing_channels
        .iter()
        .find(|channel| channel.id == channel_id)
    {
        channel
    } else {
        mark_channel_as_deleted().await?;
        return Ok(DeletionStatus::AlreadyDeleted);
    };
    // Check if the channel is marked for deletion
    let deletion_time = match channel_type {
        ChannelType::Text => {
            sqlx::query_scalar!(
                "SELECT deletion_time FROM event_series_text_channel WHERE discord_id = $1",
                channel_id.get() as i64
            )
            .fetch_one(db_connection)
            .await?
        }
        ChannelType::Voice => {
            sqlx::query_scalar!(
                "SELECT deletion_time FROM event_series_voice_channel WHERE discord_id = $1",
                channel_id.get() as i64
            )
            .fetch_one(db_connection)
            .await?
        }
    };
    let deletion_time = match deletion_time {
        Some(deletion_time) => deletion_time,
        None => return Ok(DeletionStatus::NotDeleted),
    };
    if deletion_time > chrono::Utc::now() {
        return Ok(DeletionStatus::NotDeleted);
    }
    // Delete the channel from Discord
    channel.delete(&discord_api.http, None).await?;
    // Mark the channel as deleted
    mark_channel_as_deleted().await?;
    Ok(DeletionStatus::Deleted)
}

async fn delete_marked_role(
    is_host_role: bool,
    role_id: RoleId,
    existing_roles: &[Role],
    db_connection: &sqlx::PgPool,
    discord_api: &crate::discord::CacheAndHttp,
) -> Result<DeletionStatus, crate::meetup::Error> {
    let mark_role_as_deleted = || async {
        if is_host_role {
            sqlx::query!("UPDATE event_series_host_role SET deleted = NOW() WHERE discord_id = $1 AND (deleted IS NULL OR deleted > NOW())", role_id.get() as i64).execute(db_connection).await
        } else {
            sqlx::query!("UPDATE event_series_role SET deleted = NOW() WHERE discord_id = $1 AND (deleted IS NULL OR deleted > NOW())", role_id.get() as i64).execute(db_connection).await
        }
    };
    // Check whether the role still exists on Discord
    let mut role = if let Some(role) = existing_roles.iter().find(|role| role.id == role_id) {
        role.clone()
    } else {
        mark_role_as_deleted().await?;
        return Ok(DeletionStatus::AlreadyDeleted);
    };
    // Check if the role is marked for deletion
    let deletion_time = if is_host_role {
        sqlx::query_scalar!(
            "SELECT deletion_time FROM event_series_host_role WHERE discord_id = $1",
            role_id.get() as i64
        )
        .fetch_one(db_connection)
        .await?
    } else {
        sqlx::query_scalar!(
            "SELECT deletion_time FROM event_series_role WHERE discord_id = $1",
            role_id.get() as i64
        )
        .fetch_one(db_connection)
        .await?
    };
    let deletion_time = match deletion_time {
        Some(deletion_time) => deletion_time,
        None => return Ok(DeletionStatus::NotDeleted),
    };
    if deletion_time > chrono::Utc::now() {
        return Ok(DeletionStatus::NotDeleted);
    }
    // Delete the role from Discord
    role.delete(&discord_api.http, None).await?;
    // Mark the role as deleted
    mark_role_as_deleted().await?;
    Ok(DeletionStatus::Deleted)
}
