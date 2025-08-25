#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]
pub mod db;
pub mod discord;
pub mod end_adventure;
pub mod error;
pub mod flow;
mod free_spots;
pub mod meetup;
pub mod redis;
pub mod role_shortcode;
pub mod strings;
pub mod stripe;
pub mod swissrpg;
pub mod tasks;
pub mod urls;

use base64::{engine::general_purpose, Engine as _};
use db::EventSeriesId;
pub use error::BoxedError;
use rand::Rng;
use serenity::model::id::{ChannelId, RoleId, UserId};

pub type BoxedFuture<T> = Box<dyn std::future::Future<Output = T> + Send>;

pub fn new_random_id_raw(num_bytes: u32) -> Vec<u8> {
    (0..num_bytes)
        .map(|_| rand::thread_rng().gen::<u8>())
        .collect()
}

pub fn new_random_id(num_bytes: u32) -> String {
    let random_bytes = new_random_id_raw(num_bytes);
    general_purpose::URL_SAFE_NO_PAD.encode(&random_bytes)
}

pub struct ChannelRoles {
    pub user: RoleId,
    pub host: Option<RoleId>,
}

pub async fn get_event_series_roles(
    event_series_id: db::EventSeriesId,
    db_connection: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<Option<ChannelRoles>, crate::meetup::Error> {
    let row = sqlx::query!(
        "SELECT discord_role_id, discord_host_role_id FROM event_series WHERE id = $1",
        event_series_id.0
    )
    .fetch_one(&mut **db_connection)
    .await?;
    match (row.discord_role_id, row.discord_host_role_id) {
        (Some(role), host_role) => Ok(Some(ChannelRoles {
            user: RoleId::new(role as u64),
            host: host_role.map(|role| RoleId::new(role as u64)),
        })),
        (None, None) => Ok(None),
        _ => {
            return Err(simple_error::SimpleError::new("Channel has only host role").into());
        }
    }
}

pub async fn get_channel_roles(
    channel_id: ChannelId,
    db_connection: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<Option<ChannelRoles>, crate::meetup::Error> {
    // Figure out whether this is a game channel
    let event_series_id = sqlx::query_scalar!(
        "SELECT id FROM event_series WHERE discord_text_channel_id = $1",
        channel_id.get() as i64
    )
    .fetch_optional(&mut **db_connection)
    .await?
    .map(|id| db::EventSeriesId(id));
    match event_series_id {
        None => Ok(None),
        Some(event_series_id) => get_event_series_roles(event_series_id, db_connection).await,
    }
}

pub async fn get_series_text_channel(
    event_series_id: db::EventSeriesId,
    db_connection: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<Option<ChannelId>, crate::meetup::Error> {
    let discord_text_channel_id = sqlx::query_scalar!(
        "SELECT discord_text_channel_id FROM event_series WHERE id = $1",
        event_series_id.0
    )
    .fetch_optional(&mut **db_connection)
    .await?;
    Ok(discord_text_channel_id
        .flatten()
        .map(|id| ChannelId::new(id as u64)))
}

pub async fn get_series_voice_channel(
    event_series_id: db::EventSeriesId,
    db_connection: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<Option<ChannelId>, crate::meetup::Error> {
    let discord_voice_channel_id = sqlx::query_scalar!(
        "SELECT discord_voice_channel_id FROM event_series WHERE id = $1",
        event_series_id.0
    )
    .fetch_optional(&mut **db_connection)
    .await?;
    Ok(discord_voice_channel_id
        .flatten()
        .map(|id| ChannelId::new(id as u64)))
}

pub async fn get_channel_voice_channel(
    channel_id: ChannelId,
    db_connection: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<Option<ChannelId>, crate::meetup::Error> {
    let discord_voice_channel_id = sqlx::query_scalar!(
        "SELECT discord_voice_channel_id FROM event_series WHERE discord_text_channel_id = $1",
        channel_id.get() as i64
    )
    .fetch_optional(&mut **db_connection)
    .await?;
    Ok(discord_voice_channel_id
        .flatten()
        .map(|id| ChannelId::new(id as u64)))
}

pub async fn get_channel_series(
    channel_id: ChannelId,
    db_connection: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<Option<EventSeriesId>, crate::meetup::Error> {
    let series_id = sqlx::query_scalar!(
        r#"SELECT id FROM event_series WHERE discord_text_channel_id = $1"#,
        channel_id.get() as i64
    )
    .fetch_optional(&mut **db_connection)
    .await?;
    Ok(series_id.map(|id| EventSeriesId(id)))
}

pub async fn is_game_channel(
    channel_id: ChannelId,
    db_connection: &mut sqlx::PgConnection,
) -> Result<bool, meetup::Error> {
    let is_game_channel = sqlx::query_scalar!(
        r#"SELECT COUNT(*) > 0 AS "is_game_channel!" FROM event_series_text_channel WHERE discord_id = $1"#,
        channel_id.get() as i64
    ).fetch_one(db_connection).await?;
    Ok(is_game_channel)
}

pub struct LinkingMemberMeetup {
    pub id: db::MemberId,
    pub meetup_id: u64,
    pub discord_id: Option<UserId>,
}

pub struct LinkingMemberDiscord {
    pub id: db::MemberId,
    pub meetup_id: Option<u64>,
    pub discord_id: UserId,
}

pub enum LinkingAction {
    AlreadyLinked, // Nothing was done
    NewMember,     // A new member was created
    MergedMember,  // Two existing members were merged
    Linked,        // The new link information was added to an existing member
}

pub enum LinkingResult {
    Success {
        member_id: db::MemberId,
        action: LinkingAction,
    },
    Conflict {
        member_with_meetup: LinkingMemberMeetup,
        member_with_discord: LinkingMemberDiscord,
    },
}

pub async fn link_discord_meetup(
    discord_id: UserId,
    meetup_id: u64,
    db_connection: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<LinkingResult, crate::meetup::Error> {
    // There are five cases when trying to link a Meetup and a Discord ID:
    // - the two IDs are already linked -> do nothing
    // - none are in the database -> create a new member
    // - one of them is in the database -> add the other to the existing member
    // - both are in the database without the other -> merge the two members
    // - both are in the database and at least one already has a different linking -> conflict
    let member_meetup = sqlx::query!(
        r#"SELECT id, meetup_id as "meetup_id!", discord_id FROM "member" WHERE meetup_id = $1"#,
        meetup_id as i64
    )
    .map(|row| LinkingMemberMeetup {
        id: db::MemberId(row.id),
        meetup_id: row.meetup_id as u64,
        discord_id: row.discord_id.map(|id| UserId::new(id as u64)),
    })
    .fetch_optional(&mut **db_connection)
    .await?;
    let member_discord = sqlx::query!(
        r#"SELECT id, meetup_id, discord_id as "discord_id!" FROM "member" WHERE discord_id = $1"#,
        discord_id.get() as i64
    )
    .map(|row| LinkingMemberDiscord {
        id: db::MemberId(row.id),
        meetup_id: row.meetup_id.map(|id| id as u64),
        discord_id: UserId::new(row.discord_id as u64),
    })
    .fetch_optional(&mut **db_connection)
    .await?;
    // A sanity check that should never fail:
    if let Some(member_discord) = &member_discord {
        if member_discord.discord_id != discord_id {
            panic!("Bug in implementation of link_discord_meetup: Discord IDs don't match");
        }
    }
    if let Some(member_meetup) = &member_meetup {
        if member_meetup.meetup_id != meetup_id {
            panic!("Bug in implementation of link_discord_meetup: Meetup IDs don't match");
        }
    }
    match (member_meetup, member_discord) {
        (Some(LinkingMemberMeetup { id: id1, .. }), Some(LinkingMemberDiscord { id: id2, .. }))
            if id1 == id2 =>
        {
            // Nothing to do
            Ok(LinkingResult::Success {
                member_id: id1,
                action: LinkingAction::AlreadyLinked,
            })
        }
        (None, None) => {
            // Create a new member
            let new_member_id = sqlx::query_scalar!(
                r#"INSERT INTO "member" (meetup_id, discord_id) VALUES ($1, $2) RETURNING id"#,
                meetup_id as i64,
                discord_id.get() as i64
            )
            .fetch_one(&mut **db_connection)
            .await?;
            Ok(LinkingResult::Success {
                member_id: db::MemberId(new_member_id),
                action: LinkingAction::NewMember,
            })
        }
        (Some(LinkingMemberMeetup { id, .. }), None)
        | (None, Some(LinkingMemberDiscord { id, .. })) => {
            // Add the missing ID
            let member_id = sqlx::query_scalar!(
                r#"UPDATE "member" SET meetup_id = $2, discord_id = $3 WHERE id = $1 RETURNING id"#,
                id.0,
                meetup_id as i64,
                discord_id.get() as i64
            )
            .fetch_one(&mut **db_connection)
            .await?;
            Ok(LinkingResult::Success {
                member_id: db::MemberId(member_id),
                action: LinkingAction::Linked,
            })
        }
        (
            Some(LinkingMemberMeetup {
                id: member_id_with_meetup,
                discord_id: None,
                ..
            }),
            Some(LinkingMemberDiscord {
                id: member_id_with_discord,
                meetup_id: None,
                ..
            }),
        ) => {
            // Merge the two members
            // We'll keep the one with the Discord ID and remove the one with the Meetup ID
            sqlx::query!(
                r#"UPDATE event_series_removed_host SET member_id = $2 WHERE member_id = $1"#,
                member_id_with_meetup.0,
                member_id_with_discord.0
            )
            .execute(&mut **db_connection)
            .await?;
            sqlx::query!(
                r#"UPDATE event_series_removed_user SET member_id = $2 WHERE member_id = $1"#,
                member_id_with_meetup.0,
                member_id_with_discord.0
            )
            .execute(&mut **db_connection)
            .await?;
            let hosted_events_old_member = sqlx::query!(
                r#"SELECT event_id FROM event_host WHERE member_id = $1"#,
                member_id_with_meetup.0
            )
            .map(|row| db::EventId(row.event_id))
            .fetch_all(&mut **db_connection)
            .await?;
            sqlx::query!(
                r#"DELETE FROM event_host WHERE member_id = $1"#,
                member_id_with_meetup.0
            )
            .execute(&mut **db_connection)
            .await?;
            for hosted_event_id in hosted_events_old_member {
                sqlx::query!(
                    r#"INSERT INTO event_host (event_id, member_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"#,
                    hosted_event_id.0,
                    member_id_with_discord.0
                )
                .execute(&mut **db_connection)
                .await?;
            }
            let attended_events_old_member = sqlx::query!(
                r#"SELECT event_id FROM event_participant WHERE member_id = $1"#,
                member_id_with_meetup.0
            )
            .map(|row| db::EventId(row.event_id))
            .fetch_all(&mut **db_connection)
            .await?;
            sqlx::query!(
                r#"DELETE FROM event_participant WHERE member_id = $1"#,
                member_id_with_meetup.0
            )
            .execute(&mut **db_connection)
            .await?;
            for attended_event_id in attended_events_old_member {
                sqlx::query!(
                    r#"INSERT INTO event_participant (event_id, member_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"#,
                    attended_event_id.0,
                    member_id_with_discord.0
                )
                .execute(&mut **db_connection)
                .await?;
            }
            sqlx::query!(
                r#"DELETE FROM "member" WHERE id = $1"#,
                member_id_with_meetup.0
            )
            .execute(&mut **db_connection)
            .await?;
            let member_id = sqlx::query_scalar!(
                r#"UPDATE "member" SET meetup_id = $2 WHERE id = $1 AND meetup_id IS NULL AND discord_id = $3 RETURNING id"#,
                member_id_with_discord.0,
                meetup_id as i64,
                discord_id.get() as i64
            )
            .fetch_one(&mut **db_connection)
            .await?;
            Ok(LinkingResult::Success {
                member_id: db::MemberId(member_id),
                action: LinkingAction::MergedMember,
            })
        }
        (Some(member_meetup), Some(member_discord)) => Ok(LinkingResult::Conflict {
            member_with_meetup: member_meetup,
            member_with_discord: member_discord,
        }),
    }
}

pub enum UnlinkingResult {
    Success,
    NotLinked,
}

pub async fn unlink_meetup(
    discord_id: UserId,
    db_connection: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<UnlinkingResult, crate::meetup::Error> {
    let meetup_id = sqlx::query!(
        r#"SELECT meetup_id FROM "member" WHERE discord_id = $1"#,
        discord_id.get() as i64
    )
    .map(|row| row.meetup_id.map(|id| id as u64))
    .fetch_one(&mut **db_connection)
    .await?;
    if meetup_id.is_none() {
        Ok(UnlinkingResult::NotLinked)
    } else {
        sqlx::query!(
            r#"UPDATE "member" SET meetup_id = NULL WHERE discord_id = $1"#,
            discord_id.get() as i64
        )
        .execute(&mut **db_connection)
        .await?;
        Ok(UnlinkingResult::Success)
    }
}

pub trait DefaultStr {
    fn unwrap_or_str<'s>(&'s self, default: &'s str) -> &'s str;
}

impl DefaultStr for Option<String> {
    fn unwrap_or_str<'s>(&'s self, default: &'s str) -> &'s str {
        self.as_ref().map(String::as_str).unwrap_or(default)
    }
}
