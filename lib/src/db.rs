use serenity::model::id::{ChannelId, UserId};

// macro_rules! create_wrapped_type {
//     ($Wrapper:ident, $T:ident) => {
//         #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
//         pub struct $Wrapper(pub $T);

//         impl<'r, DB: ::sqlx::Database> ::sqlx::Decode<'r, DB> for $Wrapper
//         where
//             $T: ::sqlx::Decode<'r, DB>,
//         {
//             fn decode(
//                 value: <DB as ::sqlx::database::HasValueRef<'r>>::ValueRef,
//             ) -> Result<$Wrapper, Box<dyn ::std::error::Error + 'static + Send + Sync>> {
//                 let value = <$T as ::sqlx::Decode<DB>>::decode(value)?;
//                 Ok($Wrapper(value))
//             }
//         }

//         impl<'q, DB: ::sqlx::Database> ::sqlx::Encode<'q, DB> for $Wrapper
//         where
//             $T: ::sqlx::Encode<'q, DB>,
//         {
//             #[inline]
//             fn encode(
//                 self,
//                 buf: &mut <DB as ::sqlx::database::HasArguments<'q>>::ArgumentBuffer,
//             ) -> ::sqlx::encode::IsNull {
//                 <$T as ::sqlx::Encode<DB>>::encode(self.0, buf)
//             }

//             #[inline]
//             fn encode_by_ref(
//                 &self,
//                 buf: &mut <DB as ::sqlx::database::HasArguments<'q>>::ArgumentBuffer,
//             ) -> ::sqlx::encode::IsNull {
//                 <$T as ::sqlx::Encode<DB>>::encode(self.0, buf)
//             }

//             #[inline]
//             fn produces(&self) -> Option<DB::TypeInfo> {
//                 self.0.produces()
//             }

//             #[inline]
//             fn size_hint(&self) -> usize {
//                 self.0.size_hint()
//             }
//         }
//     };
// }

// create_wrapped_type!(EventSeriesId, i32);
// create_wrapped_type!(DiscordUserId, u64);

#[derive(sqlx::Type, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[sqlx(transparent)]
pub struct EventSeriesId(pub i32);

#[derive(sqlx::Type, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[sqlx(transparent)]
pub struct EventId(pub i32);

#[derive(sqlx::Type, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[sqlx(transparent)]
pub struct MeetupEventId(pub i32);

#[derive(sqlx::Type, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[sqlx(transparent, no_pg_array)]
pub struct DiscordChannelId(pub u64);

#[derive(sqlx::Type, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[sqlx(transparent)]
pub struct MemberId(pub i32);

#[derive(sqlx::Type, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[sqlx(transparent)]
pub struct SwissRPGEventId(pub i32);

pub struct MeetupEvent {
    pub id: MeetupEventId,
    pub meetup_id: String,
    pub url: String,
    pub urlname: String,
}

pub struct SwissRPGEvent {
    pub id: SwissRPGEventId,
    pub swissrpg_id: uuid::Uuid,
    pub url: String,
}

pub struct Event {
    pub id: EventId,
    pub title: String,
    pub description: String,
    pub time: chrono::DateTime<chrono::Utc>,
    pub is_online: bool,
    pub discord_category: Option<ChannelId>,
    pub meetup_event: Option<MeetupEvent>,
    pub swissrpg_event: Option<SwissRPGEvent>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EventSource {
    Meetup,
    SwissRPG,
}

impl Event {
    pub fn source(&self) -> Option<EventSource> {
        match (&self.meetup_event, &self.swissrpg_event) {
            (Some(_), None) => Some(EventSource::Meetup),
            (None, Some(_)) => Some(EventSource::SwissRPG),
            _ => None, // Neither or both (shouldn't happen)
        }
    }
}

struct EventQueryHelper {
    event_id: i32,
    start_time: chrono::DateTime<chrono::Utc>,
    title: String,
    description: String,
    is_online: bool,
    discord_category_id: Option<i64>,
    meetup_event_id: Option<i32>,
    meetup_event_meetup_id: Option<String>,
    meetup_event_url: Option<String>,
    meetup_event_urlname: Option<String>,
    swissrpg_event_id: Option<i32>,
    swissrpg_event_swissrpg_id: Option<uuid::Uuid>,
    swissrpg_event_url: Option<String>,
}

pub struct Member {
    pub id: MemberId,
    pub meetup_id: Option<u64>,
    pub discord_id: Option<UserId>,
    pub discord_nick: Option<String>,
}

pub struct MemberWithMeetup {
    pub id: MemberId,
    pub meetup_id: u64,
    pub discord_id: Option<UserId>,
    pub discord_nick: Option<String>,
}

pub struct MemberWithDiscord {
    pub id: MemberId,
    pub meetup_id: Option<u64>,
    pub discord_id: UserId,
    pub discord_nick: Option<String>,
}

impl From<MemberWithMeetup> for Member {
    fn from(member: MemberWithMeetup) -> Self {
        Member {
            id: member.id,
            meetup_id: Some(member.meetup_id),
            discord_id: member.discord_id,
            discord_nick: member.discord_nick,
        }
    }
}

impl From<MemberWithDiscord> for Member {
    fn from(member: MemberWithDiscord) -> Self {
        Member {
            id: member.id,
            meetup_id: member.meetup_id,
            discord_id: Some(member.discord_id),
            discord_nick: member.discord_nick,
        }
    }
}

struct MemberQueryHelper {
    id: i32,
    meetup_id: Option<i64>,
    discord_id: Option<i64>,
    discord_nick: Option<String>,
}

impl From<EventQueryHelper> for Event {
    fn from(row: EventQueryHelper) -> Self {
        let meetup_event = match (
            row.meetup_event_id,
            row.meetup_event_meetup_id,
            row.meetup_event_url,
            row.meetup_event_urlname,
        ) {
            (Some(id), Some(meetup_id), Some(url), Some(urlname)) => Some(MeetupEvent {
                id: MeetupEventId(id),
                meetup_id: meetup_id,
                url: url,
                urlname: urlname,
            }),
            _ => None,
        };
        
        let swissrpg_event = match (
            row.swissrpg_event_id,
            row.swissrpg_event_swissrpg_id,
            row.swissrpg_event_url,
        ) {
            (Some(id), Some(swissrpg_id), Some(url)) => Some(SwissRPGEvent {
                id: SwissRPGEventId(id),
                swissrpg_id: swissrpg_id,
                url: url,
            }),
            _ => None,
        };
        
        Event {
            id: EventId(row.event_id),
            title: row.title,
            description: row.description,
            time: row.start_time,
            is_online: row.is_online,
            discord_category: row.discord_category_id.map(|id| ChannelId::new(id as u64)),
            meetup_event: meetup_event,
            swissrpg_event: swissrpg_event,
        }
    }
}

impl From<MemberQueryHelper> for Member {
    fn from(row: MemberQueryHelper) -> Self {
        Member {
            id: MemberId(row.id),
            meetup_id: row.meetup_id.map(|id| id as u64),
            discord_id: row.discord_id.map(|id| UserId::new(id as u64)),
            discord_nick: row.discord_nick,
        }
    }
}

pub async fn get_next_event_in_series(
    db_connection: &sqlx::PgPool,
    series_id: EventSeriesId,
) -> Result<Option<Event>, crate::meetup::Error> {
    let next_event = sqlx::query_as!(
        EventQueryHelper,
        r#"SELECT event.id as event_id, event.start_time, event.title, event.description, event.is_online, event.discord_category_id, meetup_event.id as "meetup_event_id?", meetup_event.meetup_id as "meetup_event_meetup_id?", meetup_event.url as "meetup_event_url?", meetup_event.urlname as "meetup_event_urlname?", swissrpg_event.id as "swissrpg_event_id?", swissrpg_event.swissrpg_id as "swissrpg_event_swissrpg_id?", swissrpg_event.url as "swissrpg_event_url?"
        FROM event
        LEFT OUTER JOIN meetup_event ON event.id = meetup_event.event_id
        LEFT OUTER JOIN swissrpg_event ON event.id = swissrpg_event.event_id
        WHERE event_series_id = $1 AND start_time > now() AND event.deleted IS NULL
        ORDER BY start_time
        FETCH FIRST ROW ONLY"#,
        series_id.0
    )
    .fetch_optional(db_connection)
    .await?;
    Ok(next_event.map(Into::into))
}

pub async fn get_last_event_in_series(
    db_connection: &sqlx::PgPool,
    series_id: EventSeriesId,
) -> Result<Option<Event>, crate::meetup::Error> {
    let last_event = sqlx::query_as!(
        EventQueryHelper,
        r#"SELECT event.id as event_id, event.start_time, event.title, event.description, event.is_online, event.discord_category_id, meetup_event.id as "meetup_event_id?", meetup_event.meetup_id as "meetup_event_meetup_id?", meetup_event.url as "meetup_event_url?", meetup_event.urlname as "meetup_event_urlname?", swissrpg_event.id as "swissrpg_event_id?", swissrpg_event.swissrpg_id as "swissrpg_event_swissrpg_id?", swissrpg_event.url as "swissrpg_event_url?"
        FROM event
        LEFT OUTER JOIN meetup_event ON event.id = meetup_event.event_id
        LEFT OUTER JOIN swissrpg_event ON event.id = swissrpg_event.event_id
        WHERE event_series_id = $1 AND event.deleted IS NULL
        ORDER BY start_time DESC
        FETCH FIRST ROW ONLY"#,
        series_id.0
    )
    .fetch_optional(db_connection)
    .await?;
    Ok(last_event.map(Into::into))
}

// Queries all events belonging to the specified series from latest to oldest
pub async fn get_events_for_series(
    db_connection: &sqlx::PgPool,
    series_id: EventSeriesId,
) -> Result<Vec<Event>, crate::meetup::Error> {
    let events = sqlx::query_as!(
        EventQueryHelper,
        r#"SELECT event.id as event_id, event.start_time, event.title, event.description, event.is_online, event.discord_category_id, meetup_event.id as "meetup_event_id?", meetup_event.meetup_id as "meetup_event_meetup_id?", meetup_event.url as "meetup_event_url?", meetup_event.urlname as "meetup_event_urlname?", swissrpg_event.id as "swissrpg_event_id?", swissrpg_event.swissrpg_id as "swissrpg_event_swissrpg_id?", swissrpg_event.url as "swissrpg_event_url?"
        FROM event
        LEFT OUTER JOIN meetup_event ON event.id = meetup_event.event_id
        LEFT OUTER JOIN swissrpg_event ON event.id = swissrpg_event.event_id
        WHERE event_series_id = $1 AND event.deleted IS NULL
        ORDER BY start_time DESC"#,
        series_id.0
    )
    .map(Into::into)
    .fetch_all(db_connection)
    .await?;
    Ok(events)
}

// Queries upcoming events belonging to the specified series from closest in time to furthest in the future
pub async fn get_upcoming_events_for_series(
    db_connection: &sqlx::PgPool,
    series_id: EventSeriesId,
) -> Result<Vec<Event>, crate::meetup::Error> {
    let events = sqlx::query_as!(
        EventQueryHelper,
        r#"SELECT event.id as event_id, event.start_time, event.title, event.description, event.is_online, event.discord_category_id, meetup_event.id as "meetup_event_id?", meetup_event.meetup_id as "meetup_event_meetup_id?", meetup_event.url as "meetup_event_url?", meetup_event.urlname as "meetup_event_urlname?", swissrpg_event.id as "swissrpg_event_id?", swissrpg_event.swissrpg_id as "swissrpg_event_swissrpg_id?", swissrpg_event.url as "swissrpg_event_url?"
        FROM event
        LEFT OUTER JOIN meetup_event ON event.id = meetup_event.event_id
        LEFT OUTER JOIN swissrpg_event ON event.id = swissrpg_event.event_id
        WHERE event_series_id = $1 AND event.start_time > NOW() AND event.deleted IS NULL
        ORDER BY start_time"#,
        series_id.0
    )
    .map(Into::into)
    .fetch_all(db_connection)
    .await?;
    Ok(events)
}

pub async fn get_or_create_member_for_meetup_id(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    meetup_id: u64,
) -> Result<MemberId, crate::meetup::Error> {
    let member_id = sqlx::query!(
        r#"SELECT id FROM "member" WHERE meetup_id = $1"#,
        meetup_id as i64
    )
    .map(|row| MemberId(row.id))
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(member_id) = member_id {
        Ok(member_id)
    } else {
        // Create a new member entry for this (so far unknown) Meetup user
        let member_id = sqlx::query!(
            r#"INSERT INTO "member" (meetup_id) VALUES ($1) RETURNING id"#,
            meetup_id as i64
        )
        .map(|row| MemberId(row.id))
        .fetch_one(&mut **tx)
        .await?;
        Ok(member_id)
    }
}

pub async fn get_or_create_member_for_discord_id(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    discord_id: UserId,
) -> Result<MemberId, crate::meetup::Error> {
    let member_id = sqlx::query!(
        r#"SELECT id FROM "member" WHERE discord_id = $1"#,
        discord_id.get() as i64
    )
    .map(|row| MemberId(row.id))
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(member_id) = member_id {
        Ok(member_id)
    } else {
        // Create a new member entry for this (so far unknown) Meetup user
        let member_id = sqlx::query!(
            r#"INSERT INTO "member" (discord_id) VALUES ($1) RETURNING id"#,
            discord_id.get() as i64
        )
        .map(|row| MemberId(row.id))
        .fetch_one(&mut **tx)
        .await?;
        Ok(member_id)
    }
}

// Return a list of members attending the specified events.
// If hosts is `false` returns all guests, if `hosts` is true, returns all hosts.
pub async fn get_events_participants(
    event_ids: &[EventId],
    hosts: bool,
    db_connection: &sqlx::PgPool,
) -> Result<Vec<Member>, crate::meetup::Error> {
    let event_ids: Vec<i32> = event_ids.iter().map(|id| id.0).collect();
    // Find all members RSVP'd to the specified events
    let members = if hosts {
        sqlx::query_as!(
            MemberQueryHelper,
            r#"SELECT "member".id as "id!", "member".meetup_id, "member".discord_id, "member".discord_nick
            FROM event
            INNER JOIN event_host ON event.id = event_host.event_id
            INNER JOIN "member" ON event_host.member_id = "member".id
            WHERE event.id = ANY($1)
            "#,
            &event_ids
        ).map(Into::into).fetch_all(db_connection).await?
    } else {
        sqlx::query_as!(
            MemberQueryHelper,
            r#"SELECT "member".id as "id!", "member".meetup_id, "member".discord_id, "member".discord_nick
            FROM event
            INNER JOIN event_participant ON event.id = event_participant.event_id
            INNER JOIN "member" ON event_participant.member_id = "member".id
            WHERE event.id = ANY($1)
            "#,
            &event_ids
        ).map(Into::into).fetch_all(db_connection).await?
    };
    Ok(members)
}

// Return a list of members attending the specified Meetup events.
// If hosts is `false` returns all guests, if `hosts` is true, returns all hosts.
pub async fn get_meetup_events_participants(
    meetup_event_ids: &[String],
    hosts: bool,
    db_connection: &sqlx::PgPool,
) -> Result<Vec<Member>, crate::meetup::Error> {
    // Find all members RSVP'd to the specified Meetup events
    let members = if hosts {
        sqlx::query_as!(
            MemberQueryHelper,
            r#"SELECT "member".id as "id!", "member".meetup_id, "member".discord_id, "member".discord_nick
            FROM meetup_event
            INNER JOIN event ON meetup_event.event_id = event.id
            INNER JOIN event_host ON event.id = event_host.event_id
            INNER JOIN "member" ON event_host.member_id = "member".id
            WHERE meetup_event.meetup_id = ANY($1)
            "#,
            meetup_event_ids
        ).map(Into::into).fetch_all(db_connection).await?
    } else {
        sqlx::query_as!(
            MemberQueryHelper,
            r#"SELECT "member".id as "id!", "member".meetup_id, "member".discord_id, "member".discord_nick
            FROM meetup_event
            INNER JOIN event ON meetup_event.event_id = event.id
            INNER JOIN event_participant ON event.id = event_participant.event_id
            INNER JOIN "member" ON event_participant.member_id = "member".id
            WHERE meetup_event.meetup_id = ANY($1)
            "#,
            meetup_event_ids
        ).map(Into::into).fetch_all(db_connection).await?
    };
    Ok(members)
}

// Try to translate Meetup user IDs to Members. Returns mappings from
// the Meetup ID to a Member or None if the user is unknown. The order of
// the mapping is the same as the input order.
pub async fn meetup_ids_to_members(
    meetup_user_ids: &[u64],
    db_connection: &sqlx::PgPool,
) -> Result<Vec<(u64, Option<MemberWithMeetup>)>, crate::meetup::Error> {
    let meetup_user_ids: Vec<i64> = meetup_user_ids.into_iter().map(|&id| id as i64).collect();
    let members = sqlx::query!(
        r#"SELECT query_meetup_id AS "query_meetup_id!", "member".id as "id?", "member".discord_id, "member".discord_nick
        FROM UNNEST($1::bigint[]) AS query_meetup_id
        LEFT OUTER JOIN "member" ON query_meetup_id = "member".meetup_id;"#,
        &meetup_user_ids
    )
    .map(|row| {
        if let Some(id) = row.id {
            (row.query_meetup_id as u64, Some(MemberWithMeetup {
                id: MemberId(id),
                meetup_id: row.query_meetup_id as u64,
                discord_id: row.discord_id.map(|id| UserId::new(id as u64)),
                discord_nick: row.discord_nick,
            }))
        } else {
            (row.query_meetup_id as u64, None)
        }
    })
    .fetch_all(db_connection)
    .await?;
    Ok(members)
}

// Try to translate Discord user IDs to Members. Returns mappings from
// the Discord ID to a Member or None if the user is unknown. The order of
// the mapping is the same as the input order.
pub async fn discord_ids_to_members(
    discord_user_ids: &[UserId],
    db_connection: &sqlx::PgPool,
) -> Result<Vec<(UserId, Option<MemberWithDiscord>)>, crate::meetup::Error> {
    let discord_user_ids: Vec<i64> = discord_user_ids
        .into_iter()
        .map(|&id| id.get() as i64)
        .collect();
    let members = sqlx::query!(
        r#"SELECT query_discord_id AS "query_discord_id!", "member".id as "id?", "member".meetup_id, "member".discord_nick
        FROM UNNEST($1::bigint[]) AS query_discord_id
        LEFT OUTER JOIN "member" ON query_discord_id = "member".discord_id;"#,
        &discord_user_ids
    )
    .map(|row| {
        if let Some(id) = row.id {
            (UserId::new(row.query_discord_id as u64), Some(MemberWithDiscord {
                id: MemberId(id),
                meetup_id: row.meetup_id.map(|id| id as u64),
                discord_id: UserId::new(row.query_discord_id as u64),
                discord_nick: row.discord_nick,
            }))
        } else {
            (UserId::new(row.query_discord_id as u64), None)
        }
    })
    .fetch_all(db_connection)
    .await?;
    Ok(members)
}
