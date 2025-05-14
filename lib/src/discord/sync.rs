use futures::{stream, StreamExt};
use lazy_static::lazy_static;
use redis::{self, AsyncCommands};
use serenity::{
    builder::{CreateChannel, CreateMessage, EditChannel, EditRole},
    http::CacheHttp,
    model::{
        channel::{PermissionOverwrite, PermissionOverwriteType},
        id::{ChannelId, GuildId, RoleId, UserId},
        permissions::Permissions,
    },
};
use simple_error::SimpleError;

#[cfg(feature = "bottest")]
pub mod ids {
    use super::*;
    // Test server:
    pub const GUILD_ID: GuildId = GuildId::new(601070848446824509);
    pub const BOT_ADMIN_ID: RoleId = RoleId::new(606829075226689536);
    pub const ORGANISER_ID: RoleId = RoleId::new(689914933357314090);
    pub const GAME_MASTER_ID: RoleId = RoleId::new(606913167439822987);
    pub const DICE_ROLLER_BOT_ID: Option<RoleId> = None;
    pub const ONE_SHOT_CATEGORY_IDS: &'static [ChannelId] = &[ChannelId::new(607561808429056042)];
    pub const CAMPAIGN_CATEGORY_IDS: &'static [ChannelId] = &[ChannelId::new(607561949651402772)];
    pub const VOICE_CHANNELS_CATEGORY_IDS: &'static [ChannelId] =
        &[ChannelId::new(601070848446824512)];
    pub const BOT_ALERTS_CHANNEL_ID: Option<ChannelId> = Some(ChannelId::new(650656330390175764));
    pub const FREE_SPOTS_CHANNEL_ID: Option<ChannelId> = Some(ChannelId::new(704988201038643270));
    pub const USER_TOPIC_VOICE_CHANNEL_ID: Option<ChannelId> =
        Some(ChannelId::new(807270405672140831));
    pub const ADMIN_ROLE_ID: Option<RoleId> = None;
}

#[cfg(not(feature = "bottest"))]
pub mod ids {
    use super::*;
    // SwissRPG server:
    pub const GUILD_ID: GuildId = GuildId::new(401856510709202945);
    pub const BOT_ADMIN_ID: RoleId = RoleId::new(610541498852966436);
    pub const ORGANISER_ID: RoleId = RoleId::new(539447673988841492);
    pub const GAME_MASTER_ID: RoleId = RoleId::new(412946716892069888);
    pub const DICE_ROLLER_BOT_ID: Option<RoleId> = Some(RoleId::new(600612886368223274));
    pub const ONE_SHOT_CATEGORY_IDS: &'static [ChannelId] = &[ChannelId::new(562607292176924694)];
    pub const CAMPAIGN_CATEGORY_IDS: &'static [ChannelId] = &[
        ChannelId::new(414074722259828736),
        ChannelId::new(651006290998329354),
    ];
    pub const VOICE_CHANNELS_CATEGORY_IDS: &'static [ChannelId] = &[
        ChannelId::new(401856511233753110),
        ChannelId::new(831140794952843324),
    ];
    pub const BOT_ALERTS_CHANNEL_ID: Option<ChannelId> = Some(ChannelId::new(650660608705822723));
    pub const FREE_SPOTS_CHANNEL_ID: Option<ChannelId> = Some(ChannelId::new(706131908102324345));
    pub const USER_TOPIC_VOICE_CHANNEL_ID: Option<ChannelId> =
        Some(ChannelId::new(811601700736729129));
    pub const ADMIN_ROLE_ID: Option<RoleId> = Some(RoleId::new(412927099855437825));
}

use ids::*;

use crate::db;

lazy_static! {
    static ref EVENT_NAME_REGEX: regex::Regex =
        regex::Regex::new(r"^\s*(?P<name>[^\[\(]+[^\s\[\(])").unwrap();
}

// Syncs Discord with the state of the database
pub async fn sync_discord(
    redis_connection: &mut redis::aio::MultiplexedConnection,
    db_connection: &sqlx::PgPool,
    discord_api: &super::CacheAndHttp,
    bot_id: UserId,
) -> Result<(), crate::meetup::Error> {
    let event_series_ids = sqlx::query!("SELECT id FROM event_series")
        .map(|row| db::EventSeriesId(row.id))
        .fetch_all(db_connection)
        .await?;
    let mut some_failed = false;
    for series_id in event_series_ids {
        if let Err(err) = sync_event_series(
            series_id,
            redis_connection,
            db_connection,
            discord_api,
            bot_id,
        )
        .await
        {
            some_failed = true;
            eprintln!("Discord event series syncing task failed: {}", err);
        }
    }
    if some_failed {
        Err(SimpleError::new("One or more discord event series syncs failed").into())
    } else {
        Ok(())
    }
}

/*
For each event series:
  - create a channel if it doesn't exist yet
  - store it in DB
  - create a player role if it doesn't exist yet
  - store it in DB
  - create a host role if it doesn't exist yet
  - store it in DB
  - adjust channel permission overwrites if necessary
  - find all enrolled Meetup users
  - map those Meetup users to Discord users if possible
  - assign the users (including hosts) the player role
  - assign the hosts the host role
*/
async fn sync_event_series(
    series_id: db::EventSeriesId,
    redis_connection: &mut redis::aio::MultiplexedConnection,
    db_connection: &sqlx::PgPool,
    discord_api: &super::CacheAndHttp,
    bot_id: UserId,
) -> Result<(), crate::meetup::Error> {
    // Only sync event series that have events in the future
    let next_event = match db::get_next_event_in_series(db_connection, series_id).await? {
        Some(event) => event,
        None => {
            // println!(
            //     "Event series \"{}\" seems to have no upcoming events associated with it, not \
            //      syncing to Discord",
            //     series_id
            // );
            return Ok(());
        }
    };
    // Upgrade this event series to a campaign if there is more than one event
    let num_events = sqlx::query_scalar!(
        r#"SELECT COUNT(*) as "count!" FROM event WHERE event_series_id = $1"#,
        series_id.0
    )
    .fetch_one(db_connection)
    .await?;
    if num_events > 1 {
        sqlx::query!(
            r#"UPDATE event_series SET "type" = 'campaign' WHERE id = $1 AND "type" <> 'campaign'"#,
            series_id.0
        )
        .execute(db_connection)
        .await?;
    }

    // Update this series' Discord category to match the next upcoming event's (if any)
    if let Some(discord_category) = next_event.discord_category {
        sqlx::query!(
            r#"UPDATE event_series SET discord_category_id = $2 WHERE id = $1 AND discord_category_id IS DISTINCT FROM $2"#,
            series_id.0,
            discord_category.get() as i64
        )
        .execute(db_connection)
        .await?;
    }

    // Figure out the title of this event series
    // Parse the series name from the event title
    let series_name = match EVENT_NAME_REGEX.captures(&next_event.title) {
        Some(captures) => captures.name("name").unwrap().as_str(),
        None => {
            return Err(SimpleError::new(format!(
                "Could not extract a series name from the event \"{}\"",
                next_event.title
            ))
            .into())
        }
    };
    if series_name.len() < 2 || series_name.len() > 80 {
        return Err(SimpleError::new(format!(
            "Channel name \"{}\" is too short or too long",
            series_name
        ))
        .into());
    }
    // Query the RSVPd guests and hosts
    let discord_guest_ids = sqlx::query!(
        r#"
        SELECT member.discord_id as "discord_id!"
        FROM event
        INNER JOIN event_participant ON event.id = event_participant.event_id
        INNER JOIN member ON event_participant.member_id = member.id
        WHERE event.id = $1 AND member.discord_id IS NOT NULL
        "#,
        next_event.id.0
    )
    .map(|row| UserId::new(row.discord_id as u64))
    .fetch_all(db_connection)
    .await?;
    let discord_host_ids = sqlx::query!(
        r#"
        SELECT member.discord_id as "discord_id!"
        FROM event
        INNER JOIN event_host ON event.id = event_host.event_id
        INNER JOIN member ON event_host.member_id = member.id
        WHERE event.id = $1 AND member.discord_id IS NOT NULL
        "#,
        next_event.id.0
    )
    .map(|row| UserId::new(row.discord_id as u64))
    .fetch_all(db_connection)
    .await?;

    // Step 0: Make sure that event hosts have the guild's game master role
    sync_game_master_role(series_id, db_connection, discord_api).await?;
    // Convert host IDs to user objects
    let discord_hosts: Vec<_> = stream::iter(&discord_host_ids)
        .then(|&host_id| host_id.to_user(discord_api))
        .filter_map(|res| async {
            match res {
                Ok(user) => Some(user),
                Err(err) => {
                    eprintln!(
                        "Error converting Discord host ID to Discord user object: {}",
                        err
                    );
                    None
                }
            }
        })
        .collect()
        .await;
    // Step 1: Sync the channel
    let channel_id = sync_channel(
        ChannelType::Text,
        series_name,
        series_id,
        bot_id,
        redis_connection,
        db_connection,
        discord_api,
    )
    .await?;
    // Step 2: Sync the channel's associated role
    let guest_tag = if discord_hosts.is_empty() {
        "Player".to_string()
    } else {
        itertools::join(discord_hosts.iter().map(|host| &host.name), ", ")
    };
    let guest_role_name = format!("[{}] {}", guest_tag, series_name);
    let channel_role_id = sync_role(
        &guest_role_name,
        /*is_host_role*/ false,
        series_id,
        redis_connection,
        db_connection,
        discord_api,
    )
    .await?;
    // Step 3: Sync the channel's associated host role
    // let host_role_name = format!("[Host] {}", series_name);
    // let channel_host_role_id = sync_role(
    //     &host_role_name,
    //     /*is_host_role*/ true,
    //     channel_id,
    //     redis_connection,
    //     discord_api,
    // )?;
    // Step 4: Sync the channel permissions
    if let Err(err) = sync_channel_permissions(
        channel_id,
        ChannelType::Text,
        channel_role_id,
        &discord_host_ids,
        bot_id,
        discord_api,
    )
    .await
    {
        eprintln!(
            "Error in sync_channel_permissions (for text channel):\n{:#?}",
            err
        );
    }
    // Step 5: If this is an online campaign, also create a voice channel
    let voice_channel_id = if next_event.is_online {
        match sync_channel(
            ChannelType::Voice,
            series_name,
            series_id,
            bot_id,
            redis_connection,
            db_connection,
            discord_api,
        )
        .await
        {
            Err(err) => {
                eprintln!("Error in sync_channel (for voice channel):\n{:#?}", err);
                None
            }
            Ok(voice_channel_id) => {
                if let Err(err) = sync_channel_permissions(
                    voice_channel_id,
                    ChannelType::Voice,
                    channel_role_id,
                    &discord_host_ids,
                    bot_id,
                    discord_api,
                )
                .await
                {
                    eprintln!(
                        "Error in sync_channel_permissions (for voice channel):\n{:#?}",
                        err
                    );
                }
                Some(voice_channel_id)
            }
        }
    } else {
        None
    };
    // Step 5: Sync RSVP'd users
    // sync_user_role_assignments(
    //     &discord_host_ids,
    //     channel_id,
    //     channel_host_role_id,
    //     /*is_host_role*/ true,
    //     redis_connection,
    //     discord_api,
    // )?;
    sync_role_assignments_permissions(
        &discord_guest_ids,
        &discord_host_ids,
        series_id,
        channel_id,
        voice_channel_id,
        channel_role_id,
        db_connection,
        discord_api,
    )
    .await?;
    // Step 6: Keep the channel's topic up-to-date
    sync_channel_topic(channel_id, &next_event, discord_api).await?;
    sync_channel_category(
        series_id,
        ChannelType::Text,
        &next_event,
        channel_id,
        db_connection,
        discord_api,
    )
    .await?;
    if let Some(voice_channel_id) = voice_channel_id {
        sync_channel_category(
            series_id,
            ChannelType::Voice,
            &next_event,
            voice_channel_id,
            db_connection,
            discord_api,
        )
        .await?;
    }
    Ok(())
}

async fn sync_role(
    role_name: &str,
    is_host_role: bool,
    event_series: db::EventSeriesId,
    redis_connection: &mut redis::aio::MultiplexedConnection,
    db_connection: &sqlx::PgPool,
    discord_api: &super::CacheAndHttp,
) -> Result<RoleId, crate::meetup::Error> {
    let max_retries: u32 = 1;
    let mut current_num_try: u32 = 0;
    loop {
        if current_num_try > max_retries {
            return Err(SimpleError::new("Role sync failed, max retries reached").into());
        }
        current_num_try += 1;
        let role = sync_role_impl(
            role_name,
            is_host_role,
            event_series,
            redis_connection,
            db_connection,
            discord_api,
        )
        .await?;
        // Make sure that the role ID that was returned actually exists on Discord
        // First, check the cache
        let role_exists = match GUILD_ID.to_guild_cached(&discord_api.cache) {
            Some(guild) => guild.roles.contains_key(&role),
            None => false,
        };
        // If it was not in the cache, check Discord
        let role_exists = if role_exists {
            true
        } else {
            let guild_roles = discord_api.http().get_guild_roles(GUILD_ID).await?;
            guild_roles.iter().any(|guild_role| guild_role.id == role)
        };
        if !role_exists {
            // This role does not exist on Discord
            // Delete it from the DB and retry
            if is_host_role {
                sqlx::query!(
                    "UPDATE event_series SET discord_host_role_id = NULL WHERE id = $1 AND \
                     discord_host_role_id = $2",
                    event_series.0,
                    role.get() as i64
                )
                .execute(db_connection)
                .await?;
            } else {
                sqlx::query!(
                    "UPDATE event_series SET discord_role_id = NULL WHERE id = $1 AND \
                     discord_role_id = $2",
                    event_series.0,
                    role.get() as i64
                )
                .execute(db_connection)
                .await?;
            }
            continue;
        } else {
            // The role exists on Discord, so everything is good
            return Ok(role);
        }
    }
}

async fn sync_role_impl(
    role_name: &str,
    is_host_role: bool,
    series_id: db::EventSeriesId,
    redis_connection: &mut redis::aio::MultiplexedConnection,
    db_connection: &sqlx::PgPool,
    discord_api: &super::CacheAndHttp,
) -> Result<RoleId, crate::meetup::Error> {
    let mut tx = db_connection.begin().await?;
    // Check if the role already exists
    let role_id = if is_host_role {
        sqlx::query_scalar!(
            r#"SELECT discord_host_role_id FROM event_series WHERE id = $1 FOR UPDATE"#,
            series_id.0
        )
        .fetch_one(&mut *tx)
        .await?
    } else {
        sqlx::query_scalar!(
            r#"SELECT discord_role_id FROM event_series WHERE id = $1 FOR UPDATE"#,
            series_id.0
        )
        .fetch_one(&mut *tx)
        .await?
    };
    let role_id = role_id.map(|id| RoleId::new(id as u64));
    if let Some(role_id) = role_id {
        return Ok(role_id);
    }
    // The role doesn't exist yet -> try to create it
    let role_builder = EditRole::new()
        .name(role_name)
        .colour(serenity::all::Colour::BLUE)
        .permissions(Permissions::empty());
    let temp_channel_role = GUILD_ID
        .create_role(discord_api.http(), role_builder)
        .await?;
    println!(
        "Discord event sync: created new temporary channel role {} \"{}\"",
        temp_channel_role.id.get(),
        &temp_channel_role.name
    );
    let insert_query = if is_host_role {
        sqlx::query!(
            "INSERT INTO event_series_host_role (discord_id) VALUES ($1)",
            temp_channel_role.id.get() as i64
        )
    } else {
        sqlx::query!(
            "INSERT INTO event_series_role (discord_id) VALUES ($1)",
            temp_channel_role.id.get() as i64
        )
    };
    let update_query = if is_host_role {
        sqlx::query!(
            "UPDATE event_series SET discord_host_role_id = $2 WHERE id = $1",
            series_id.0,
            temp_channel_role.id.get() as i64
        )
    } else {
        sqlx::query!(
            "UPDATE event_series SET discord_role_id = $2 WHERE id = $1",
            series_id.0,
            temp_channel_role.id.get() as i64
        )
    };
    let mut any_err = insert_query.execute(&mut *tx).await.err();
    if any_err.is_none() {
        any_err = update_query.execute(&mut *tx).await.err();
    }
    if any_err.is_none() {
        any_err = tx.commit().await.err();
    }
    // In case the transaction failed delete the temporary role from Discord
    match any_err {
        Some(err) => {
            println!("Trying to delete temporary channel role");
            match discord_api
                .http()
                .delete_role(
                    GUILD_ID,
                    temp_channel_role.id,
                    Some("sync_role_impl transaction failed"),
                )
                .await
            {
                Ok(_) => println!("Successfully deleted temporary channel role"),
                Err(_) => {
                    eprintln!(
                        "Could not delete temporary channel role {}",
                        temp_channel_role.id.get()
                    );
                    // Try to persist the information to Redis that we have an orphaned role now
                    match redis_connection
                        .sadd("orphaned_discord_roles", temp_channel_role.id.get())
                        .await
                    {
                        Err(_) => eprintln!(
                            "Could not record orphaned channel role {}",
                            temp_channel_role.id.get()
                        ),
                        Ok(()) => {
                            println!(
                                "Recorded orphaned channel role {}",
                                temp_channel_role.id.get()
                            )
                        }
                    }
                }
            }
            Err(err.into())
        }
        None => {
            println!("Persisted new channel role {}", temp_channel_role.id.get());
            // Return the new channel role
            Ok(temp_channel_role.id)
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub(crate) enum ChannelType {
    Text,
    Voice,
}

async fn sync_channel(
    channel_type: ChannelType,
    channel_name: &str,
    event_series_id: db::EventSeriesId,
    bot_id: UserId,
    redis_connection: &mut redis::aio::MultiplexedConnection,
    db_connection: &sqlx::PgPool,
    discord_api: &super::CacheAndHttp,
) -> Result<ChannelId, crate::meetup::Error> {
    let max_retries: u32 = 1;
    let mut current_num_try: u32 = 0;
    loop {
        if current_num_try > max_retries {
            return Err(SimpleError::new("Channel sync failed, max retries reached").into());
        }
        current_num_try += 1;
        let channel = sync_channel_impl(
            channel_type,
            channel_name,
            event_series_id,
            bot_id,
            redis_connection,
            db_connection,
            discord_api,
        )
        .await?;
        // Make sure that the channel ID that was returned actually exists on Discord
        let channel_exists = match channel.to_channel(discord_api).await {
            Ok(_) => true,
            Err(err) => {
                if let serenity::Error::Http(http_err) = &err {
                    if let serenity::http::HttpError::UnsuccessfulRequest(response) = http_err {
                        if response.status_code == serenity::http::StatusCode::NOT_FOUND {
                            false
                        } else {
                            return Err(err.into());
                        }
                    } else {
                        return Err(err.into());
                    }
                } else {
                    return Err(err.into());
                }
            }
        };
        if !channel_exists {
            // This channel does not exist on Discord
            // Delete it from the DB and retry
            match channel_type {
                ChannelType::Text => {
                    sqlx::query!(
                        "UPDATE event_series SET discord_text_channel_id = NULL WHERE id = $1 AND \
                         discord_text_channel_id = $2",
                        event_series_id.0,
                        channel.get() as i64
                    )
                    .execute(db_connection)
                    .await?
                }
                ChannelType::Voice => {
                    sqlx::query!(
                        "UPDATE event_series SET discord_voice_channel_id = NULL WHERE id = $1 \
                         AND discord_voice_channel_id = $2",
                        event_series_id.0,
                        channel.get() as i64
                    )
                    .execute(db_connection)
                    .await?
                }
            };
            continue;
        } else {
            // The channel exists on Discord, so everything is good
            return Ok(channel);
        }
    }
}

async fn sync_channel_impl(
    channel_type: ChannelType,
    channel_name: &str,
    event_series_id: db::EventSeriesId,
    bot_id: UserId,
    redis_connection: &mut redis::aio::MultiplexedConnection,
    db_connection: &sqlx::PgPool,
    discord_api: &super::CacheAndHttp,
) -> Result<ChannelId, crate::meetup::Error> {
    let mut tx = db_connection.begin().await?;
    // Check if the channel already exists
    let channel_id = match channel_type {
        ChannelType::Text => {
            sqlx::query_scalar!(
                "SELECT discord_text_channel_id FROM event_series WHERE id = $1 FOR UPDATE",
                event_series_id.0
            )
            .fetch_one(&mut *tx)
            .await?
        }
        ChannelType::Voice => {
            sqlx::query_scalar!(
                "SELECT discord_voice_channel_id FROM event_series WHERE id = $1 FOR UPDATE",
                event_series_id.0
            )
            .fetch_one(&mut *tx)
            .await?
        }
    };
    let channel_id = channel_id.map(|id| ChannelId::new(id as u64));
    if let Some(channel_id) = channel_id {
        return Ok(channel_id);
    }
    // The channel doesn't exist yet -> try to create it
    // The @everyone role has the same id as the guild
    let permission_overwrites = match channel_type {
        ChannelType::Text => vec![
            PermissionOverwrite {
                allow: Permissions::empty(),
                deny: Permissions::VIEW_CHANNEL,
                kind: PermissionOverwriteType::Role(GUILD_ID.everyone_role()),
            },
            PermissionOverwrite {
                allow: Permissions::VIEW_CHANNEL,
                deny: Permissions::empty(),
                kind: PermissionOverwriteType::Member(bot_id),
            },
        ],
        ChannelType::Voice => vec![
            PermissionOverwrite {
                allow: Permissions::empty(),
                deny: Permissions::CONNECT,
                kind: PermissionOverwriteType::Role(GUILD_ID.everyone_role()),
            },
            PermissionOverwrite {
                allow: Permissions::CONNECT,
                deny: Permissions::empty(),
                kind: PermissionOverwriteType::Member(bot_id),
            },
        ],
    };
    let channel_builder = CreateChannel::new(channel_name)
        .kind(match channel_type {
            ChannelType::Text => serenity::model::channel::ChannelType::Text,
            ChannelType::Voice => serenity::model::channel::ChannelType::Voice,
        })
        .permissions(permission_overwrites);
    let temp_channel = GUILD_ID
        .create_channel(discord_api.http(), channel_builder)
        .await?;
    println!(
        "Discord event sync: created new temporary channel {} \"{}\"",
        temp_channel.id.get(),
        &temp_channel.name
    );
    let insert_query = match channel_type {
        ChannelType::Text => {
            sqlx::query!(
                "INSERT INTO event_series_text_channel (discord_id) VALUES ($1)",
                temp_channel.id.get() as i64
            )
        }
        ChannelType::Voice => {
            sqlx::query!(
                "INSERT INTO event_series_voice_channel (discord_id) VALUES ($1)",
                temp_channel.id.get() as i64
            )
        }
    };
    let update_query = match channel_type {
        ChannelType::Text => {
            sqlx::query!(
                "UPDATE event_series SET discord_text_channel_id = $2 WHERE id = $1",
                event_series_id.0,
                temp_channel.id.get() as i64
            )
        }
        ChannelType::Voice => {
            sqlx::query!(
                "UPDATE event_series SET discord_voice_channel_id = $2 WHERE id = $1",
                event_series_id.0,
                temp_channel.id.get() as i64
            )
        }
    };
    let mut any_err = insert_query.execute(&mut *tx).await.err();
    if any_err.is_none() {
        any_err = update_query.execute(&mut *tx).await.err();
    }
    if any_err.is_none() {
        any_err = tx.commit().await.err();
    }
    // In case the transaction failed delete the temporary channel from Discord
    match any_err {
        Some(err) => {
            println!("Trying to delete temporary channel");
            match discord_api
                .http()
                .delete_channel(
                    temp_channel.id,
                    Some("sync_channel_impl transaction failed"),
                )
                .await
            {
                Ok(_) => println!("Successfully deleted temporary channel"),
                Err(_) => {
                    eprintln!(
                        "Could not delete temporary channel {}",
                        temp_channel.id.get()
                    );
                    // Try to persist the information to Redis that we have an orphaned channel now
                    let redis_orphaned_channels_key = match channel_type {
                        ChannelType::Text => "orphaned_discord_channels",
                        ChannelType::Voice => "orphaned_discord_voice_channels",
                    };
                    match redis_connection
                        .sadd(redis_orphaned_channels_key, temp_channel.id.get())
                        .await
                    {
                        Err(_) => {
                            eprintln!(
                                "Could not record orphaned channel {}",
                                temp_channel.id.get()
                            )
                        }
                        Ok(()) => println!("Recorded orphaned channel {}", temp_channel.id.get()),
                    }
                }
            }
            Err(err.into())
        }
        None => {
            println!("Persisted new channel {}", temp_channel.id.get());
            // Return the new channel
            Ok(temp_channel.id)
        }
    }
}

// Makes sure that the Discord channel has the appropriate permission
// overwrites for the channel's role and host role.
// Specifically does not remove any additional permission overwrites
// that the channel might have.
async fn sync_channel_permissions(
    channel_id: ChannelId,
    channel_type: ChannelType,
    role_id: RoleId,
    discord_host_ids: &[UserId],
    bot_id: UserId,
    discord_api: &super::CacheAndHttp,
) -> Result<(), crate::meetup::Error> {
    // Make this channel private.
    // This is achieved by denying @everyone the VIEW_CHANNEL permission
    // but allowing the new role the VIEW_CHANNEL permission.
    // see: https://support.discordapp.com/hc/en-us/articles/206143877-How-do-I-set-up-a-Role-Exclusive-channel-
    let permission_overwrites = match channel_type {
        ChannelType::Text => {
            let mut permission_overwrites = vec![
                PermissionOverwrite {
                    allow: Permissions::empty(),
                    deny: Permissions::VIEW_CHANNEL,
                    kind: PermissionOverwriteType::Role(GUILD_ID.everyone_role()),
                },
                PermissionOverwrite {
                    allow: Permissions::VIEW_CHANNEL,
                    deny: Permissions::empty(),
                    kind: PermissionOverwriteType::Member(bot_id),
                },
                PermissionOverwrite {
                    allow: Permissions::VIEW_CHANNEL,
                    deny: Permissions::empty(),
                    kind: PermissionOverwriteType::Role(role_id),
                },
                PermissionOverwrite {
                    allow: Permissions::VIEW_CHANNEL
                        | Permissions::MENTION_EVERYONE
                        | Permissions::MANAGE_MESSAGES,
                    deny: Permissions::empty(),
                    kind: PermissionOverwriteType::Role(ORGANISER_ID),
                },
            ];
            if let Some(dice_roller_bot_id) = DICE_ROLLER_BOT_ID {
                permission_overwrites.push(PermissionOverwrite {
                    allow: Permissions::VIEW_CHANNEL,
                    deny: Permissions::empty(),
                    kind: PermissionOverwriteType::Role(dice_roller_bot_id),
                });
            }
            for &host_id in discord_host_ids {
                permission_overwrites.push(PermissionOverwrite {
                    allow: Permissions::VIEW_CHANNEL
                        | Permissions::MENTION_EVERYONE
                        | Permissions::MANAGE_MESSAGES,
                    deny: Permissions::empty(),
                    kind: PermissionOverwriteType::Member(host_id),
                });
            }
            permission_overwrites
        }
        ChannelType::Voice => {
            let mut permission_overwrites = vec![
                PermissionOverwrite {
                    allow: Permissions::empty(),
                    deny: Permissions::VIEW_CHANNEL | Permissions::CONNECT,
                    kind: PermissionOverwriteType::Role(GUILD_ID.everyone_role()),
                },
                PermissionOverwrite {
                    allow: Permissions::VIEW_CHANNEL | Permissions::CONNECT,
                    deny: Permissions::empty(),
                    kind: PermissionOverwriteType::Member(bot_id),
                },
                PermissionOverwrite {
                    allow: Permissions::VIEW_CHANNEL | Permissions::CONNECT,
                    deny: Permissions::empty(),
                    kind: PermissionOverwriteType::Role(role_id),
                },
                PermissionOverwrite {
                    allow: Permissions::VIEW_CHANNEL
                        | Permissions::CONNECT
                        | Permissions::MOVE_MEMBERS
                        | Permissions::MUTE_MEMBERS
                        | Permissions::DEAFEN_MEMBERS,
                    deny: Permissions::empty(),
                    kind: PermissionOverwriteType::Role(ORGANISER_ID),
                },
            ];
            for &host_id in discord_host_ids {
                permission_overwrites.push(PermissionOverwrite {
                    allow: Permissions::CONNECT
                        | Permissions::MUTE_MEMBERS
                        | Permissions::DEAFEN_MEMBERS
                        | Permissions::MOVE_MEMBERS
                        | Permissions::PRIORITY_SPEAKER,
                    deny: Permissions::empty(),
                    kind: PermissionOverwriteType::Member(host_id),
                });
            }
            permission_overwrites
        }
    };
    for permission_overwrite in permission_overwrites {
        channel_id
            .create_permission(discord_api.http(), permission_overwrite)
            .await?;
    }
    Ok(())
}

async fn sync_role_assignments_permissions(
    discord_user_ids: &[UserId],
    discord_host_ids: &[UserId],
    series_id: db::EventSeriesId,
    channel_id: ChannelId,
    voice_channel_id: Option<ChannelId>,
    user_role: RoleId,
    db_connection: &sqlx::PgPool,
    discord_api: &super::CacheAndHttp,
) -> Result<(), crate::meetup::Error> {
    // Check whether any users have manually removed roles and don't add them back
    // Don't automatically assign the user role to user that have been
    // manually removed from a channel
    let ignore_discord_user_ids = sqlx::query!(
        r#"SELECT member_id FROM event_series_removed_user WHERE event_series_id = $1"#,
        series_id.0
    )
    .map(|row| UserId::new(row.member_id as u64))
    .fetch_all(db_connection)
    .await?;
    // Don't automatically assign the host role to users that have either
    // been manually removed as a host or as a user from a channel
    let ignore_discord_host_ids = sqlx::query!(
        r#"SELECT member_id FROM event_series_removed_host WHERE event_series_id = $1"#,
        series_id.0
    )
    .map(|row| UserId::new(row.member_id as u64))
    .fetch_all(db_connection)
    .await?;
    // Assign the role to the Discord users
    let mut newly_added_user_ids = vec![];
    for &user_id in discord_user_ids {
        if ignore_discord_user_ids.contains(&user_id) {
            continue;
        }
        match user_id.to_user(discord_api).await {
            Ok(user) => match user.has_role(discord_api, GUILD_ID, user_role).await {
                Ok(has_role) => {
                    if !has_role {
                        match discord_api
                            .http()
                            .add_member_role(
                                GUILD_ID,
                                user_id,
                                user_role,
                                Some("Automatic role assignment due to event participation"),
                            )
                            .await
                        {
                            Ok(_) => {
                                println!("Assigned user {} to role {}", user_id, user_role);
                                newly_added_user_ids.push(user_id);
                            }
                            Err(err) => eprintln!(
                                "Could not assign user {} to role {}: {}",
                                user_id, user_role, err
                            ),
                        }
                    }
                }
                Err(err) => eprintln!(
                    "Could not figure out whether the user {} already has role {}: {}",
                    user.id, user_role, err
                ),
            },
            Err(err) => eprintln!("Could not find the user {}: {}", user_id, err),
        }
    }
    // Assign direct permissions to hosts
    let mut newly_added_host_ids = vec![];
    for &host_id in discord_host_ids {
        if ignore_discord_host_ids.contains(&host_id) {
            continue;
        }
        // Assign text channel permissions
        let new_permissions = Permissions::VIEW_CHANNEL
            | Permissions::MENTION_EVERYONE
            | Permissions::MANAGE_MESSAGES;
        match crate::discord::add_channel_user_permissions(
            discord_api,
            channel_id,
            host_id,
            new_permissions,
        )
        .await
        {
            Ok(true) => {
                println!("Assigned user {} host permissions in text channel", host_id);
                newly_added_host_ids.push(host_id);
            }
            Err(err) => eprintln!(
                "Could not assign user {} host permissions in text channel:\n{:#?}",
                host_id, err
            ),
            _ => (),
        }
        // Also assign rights in the possibly existing voice channel
        if let Some(voice_channel_id) = voice_channel_id {
            let new_permissions = Permissions::VIEW_CHANNEL
                | Permissions::CONNECT
                | Permissions::MUTE_MEMBERS
                | Permissions::DEAFEN_MEMBERS
                | Permissions::MOVE_MEMBERS
                | Permissions::PRIORITY_SPEAKER;
            match crate::discord::add_channel_user_permissions(
                discord_api,
                voice_channel_id,
                host_id,
                new_permissions,
            )
            .await
            {
                Ok(true) => {
                    println!(
                        "Assigned user {} host permissions in voice channel",
                        host_id
                    );
                }
                Err(err) => eprintln!(
                    "Could not assign user {} host permissions in voice channel:\n{:#?}",
                    host_id, err
                ),
                _ => (),
            }
        }
    }
    // Announce the newly added users
    if !newly_added_host_ids.is_empty() {
        let message_builder = CreateMessage::new()
            .content(crate::strings::CHANNEL_ADDED_HOSTS(&newly_added_host_ids));
        if let Err(err) = channel_id
            .send_message(&discord_api.http, message_builder)
            .await
        {
            eprintln!(
                "Could not announce new hosts in channel {}:\n{:#?}",
                channel_id, err
            );
        };
    }
    if !newly_added_user_ids.is_empty() {
        let message_builder = CreateMessage::new()
            .content(crate::strings::CHANNEL_ADDED_PLAYERS(&newly_added_user_ids));
        if let Err(err) = channel_id
            .send_message(&discord_api.http, message_builder)
            .await
        {
            eprintln!(
                "Could not announce new players in channel {}:\n{:#?}",
                channel_id, err
            );
        };
    }
    Ok(())
}

async fn sync_game_master_role(
    event_series_id: db::EventSeriesId,
    db_connection: &sqlx::PgPool,
    discord_api: &super::CacheAndHttp,
) -> Result<(), crate::meetup::Error> {
    // Find all Discord users that are a host for any of the events in this series
    let discord_host_ids = sqlx::query!(
        r#"
        SELECT member.discord_id as "discord_id!"
        FROM event_series
        INNER JOIN event ON event_series.id = event.event_series_id
        INNER JOIN event_host ON event.id = event_host.event_id
        INNER JOIN member ON event_host.member_id = member.id
        WHERE event_series.id = $1 AND member.discord_id IS NOT NULL"#,
        event_series_id.0
    )
    .map(|row| UserId::new(row.discord_id as u64))
    .fetch_all(db_connection)
    .await?;
    // Assign the Game Master role to the hosts
    for host_id in discord_host_ids {
        match host_id.to_user(discord_api).await {
            Ok(user) => match user.has_role(discord_api, GUILD_ID, GAME_MASTER_ID).await {
                Ok(has_role) => {
                    if !has_role {
                        match discord_api
                            .http()
                            .add_member_role(
                                GUILD_ID,
                                host_id,
                                GAME_MASTER_ID,
                                Some("Automatic role assignment due to being a game master"),
                            )
                            .await
                        {
                            Ok(_) => println!("Assigned user {} to the game master role", host_id),
                            Err(err) => eprintln!(
                                "Could not assign user {} to the game master role: {}",
                                host_id, err
                            ),
                        }
                    }
                }
                Err(err) => eprintln!(
                    "Could not figure out whether the user {} already has the game master role: {}",
                    user.id, err
                ),
            },
            Err(err) => eprintln!("Could not find the host user {}: {}", host_id, err),
        }
    }
    Ok(())
}

async fn sync_channel_topic(
    channel_id: ChannelId,
    next_event: &db::Event,
    discord_api: &super::CacheAndHttp,
) -> Result<(), crate::meetup::Error> {
    // Sync the topic
    let topic = match &next_event.meetup_event {
        Some(meetup_event) => format!("Next session: {}", meetup_event.url),
        None => format!(
            "Next Session: {}",
            next_event
                .time
                .with_timezone(&chrono_tz::Europe::Zurich)
                .format("%d.%m.%Y %H:%M")
        ),
    };
    let channel = channel_id.to_channel(discord_api).await?;
    if let serenity::model::channel::Channel::Guild(channel) = channel {
        let topic_needs_update = if let Some(current_topic) = channel.topic {
            current_topic != topic
        } else {
            true
        };
        if topic_needs_update {
            channel_id
                .edit(&discord_api.http, EditChannel::new().topic(topic))
                .await?;
        }
    }
    Ok(())
}

async fn sync_channel_category(
    series_id: db::EventSeriesId,
    channel_type: ChannelType,
    next_event: &db::Event,
    channel_id: ChannelId,
    db_connection: &sqlx::PgPool,
    discord_api: &super::CacheAndHttp,
) -> Result<(), crate::meetup::Error> {
    // Sync the category
    let event_series_type = sqlx::query_scalar!(
        r#"SELECT "type" from event_series WHERE id = $1"#,
        series_id.0
    )
    .fetch_one(db_connection)
    .await?;
    let mut categories = if let Some(special_category) = next_event.discord_category {
        vec![special_category]
    } else {
        vec![]
    };
    match channel_type {
        ChannelType::Text => match event_series_type.as_str() {
            "campaign" => categories.extend_from_slice(CAMPAIGN_CATEGORY_IDS),
            "adventure" => categories.extend_from_slice(ONE_SHOT_CATEGORY_IDS),
            _ => {
                eprintln!(
                    "Event series {} does not have a type of 'campaign' or 'adventure'",
                    series_id.0
                );
                categories.extend_from_slice(CAMPAIGN_CATEGORY_IDS)
            }
        },
        ChannelType::Voice => categories.extend_from_slice(VOICE_CHANNELS_CATEGORY_IDS),
    }
    let channel = channel_id.to_channel(discord_api).await?;
    if let serenity::model::channel::Channel::Guild(channel) = channel {
        let category_needs_update = match channel.parent_id {
            Some(channel_category) => {
                if let Some(special_category) = next_event.discord_category {
                    special_category != channel_category
                } else {
                    !categories.contains(&channel_category)
                }
            }
            None => true,
        };
        if category_needs_update {
            // Try the categories in order and put the channel in the first
            // one that works. Meetup has an undocumented limit of 50 channels
            // per category, so an error will be returned if the category is full.
            for category in categories {
                if let Ok(_) = channel_id
                    .edit(
                        &discord_api.http,
                        EditChannel::new().category(Some(category)),
                    )
                    .await
                {
                    break;
                }
            }
        }
    }
    Ok(())
}
