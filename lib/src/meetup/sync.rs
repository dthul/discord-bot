use futures_util::{lock::Mutex, stream::StreamExt};
use lazy_static::lazy_static;
use simple_error::SimpleError;
use std::sync::Arc;

use crate::{db, meetup::newapi::IsOnline, DefaultStr};

pub const NEW_ADVENTURE_PATTERN: &'static str = r"(?i)\\?\[\s*new\s*adventure\s*\\?\]";
pub const NEW_CAMPAIGN_PATTERN: &'static str = r"(?i)\\?\[\s*new\s*campaign\s*\\?\]";
pub const EVENT_SERIES_PATTERN: &'static str =
    r"(?i)\\?\[\s*campaign\s*(?P<event_id>[a-zA-Z0-9]+)\s*\\?\]";
pub const CHANNEL_PATTERN: &'static str = r"(?i)\\?\[\s*channel\s*(?P<channel_id>[0-9]+)\s*\\?\]";
pub const SESSION_PATTERN: &'static str = r"(?i)\s*session\s*(?P<number>[0-9]+)";
pub const ONLINE_PATTERN: &'static str = r"(?i)\\?\[\s*online\s*\\?\]";
pub const ROLE_PATTERN: &'static str = r"(?i)\\?\[\s*role\s*(?P<role_id>[0-9]+)\s*\\?\]";
pub const CATEGORY_PATTERN: &'static str =
    r"(?i)\\?\[\s*category\s*(?P<category_id>[0-9]+)\s*\\?\]";

lazy_static! {
    pub static ref NEW_ADVENTURE_REGEX: regex::Regex =
        regex::Regex::new(NEW_ADVENTURE_PATTERN).unwrap();
    pub static ref NEW_CAMPAIGN_REGEX: regex::Regex =
        regex::Regex::new(NEW_CAMPAIGN_PATTERN).unwrap();
    pub static ref EVENT_SERIES_REGEX: regex::Regex =
        regex::Regex::new(EVENT_SERIES_PATTERN).unwrap();
    pub static ref CHANNEL_REGEX: regex::Regex = regex::Regex::new(CHANNEL_PATTERN).unwrap();
    pub static ref SESSION_REGEX: regex::Regex = regex::Regex::new(SESSION_PATTERN).unwrap();
    pub static ref ONLINE_REGEX: regex::Regex = regex::Regex::new(ONLINE_PATTERN).unwrap();
    pub static ref ROLE_REGEX: regex::Regex = regex::Regex::new(ROLE_PATTERN).unwrap();
    pub static ref CATEGORY_REGEX: regex::Regex = regex::Regex::new(CATEGORY_PATTERN).unwrap();
}

// TODO: Introduce a type like "Meetup connection" that contains
// an Arc<Mutex<Option<MeetupClient>>> internally and has the same
// methods as MeetupClient (so we don't need to match on the Option
// every time we want to use the client)
pub async fn sync_task(
    meetup_client: Arc<Mutex<Option<Arc<super::newapi::AsyncClient>>>>,
    db_connection: &sqlx::PgPool,
) -> Result<crate::free_spots::EventCollector, super::Error> {
    let meetup_client = {
        let guard = meetup_client.lock().await;
        match *guard {
            Some(ref meetup_client) => meetup_client.clone(),
            None => return Err(SimpleError::new("Meetup API unavailable").into()),
        }
        // The Mutex guard will be dropped here
    };
    let upcoming_events = meetup_client.get_upcoming_events_all_groups();
    futures::pin_mut!(upcoming_events);
    // Sync events
    // For loops for streams not supported (yet?)
    // While looping over the upcoming events, we also keep information about
    // free spots. This information will be posted to Discord.
    let mut event_collector = crate::free_spots::EventCollector::new();
    while let Some(event) = upcoming_events.next().await {
        match event {
            Err(err) => eprintln!("Couldn't query upcoming event: {}", err),
            Ok(event) => {
                let common_event = crate::common_event::CommonEventDetails::from(event.clone());
                event_collector.add_event(common_event);
                match sync_event(event, db_connection).await {
                    Err(err) => eprintln!("Event sync failed: {}", err),
                    _ => (),
                }
            }
        }
    }
    // Sync event series with upcoming Meetup events
    let meetup_active_event_series = sqlx::query!(
        r#"SELECT event_series.id
        FROM event_series
        INNER JOIN event ON event_series.id = event.event_series_id
        INNER JOIN meetup_event ON event.id = meetup_event.event_id
        WHERE event.start_time > NOW() AND event.deleted IS NULL
        GROUP BY event_series.id"#
    )
    .map(|row| db::EventSeriesId(row.id))
    .fetch_all(db_connection)
    .await?;
    for series_id in meetup_active_event_series {
        match sync_event_series(series_id, meetup_client.as_ref(), db_connection).await {
            Err(err) => eprintln!("Series sync failed: {}", err),
            _ => (),
        };
        // Add a 250ms delay between each item as a naive rate limit for the Meetup API
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    Ok(event_collector)
}

// This function is supposed to be idempotent, so calling it with the same
// event is fine.
pub async fn sync_event(
    event: super::newapi::UpcomingEventDetails,
    db_connection: &sqlx::PgPool,
) -> Result<(), super::Error> {
    let description = event.description;
    let title = event.title;
    let is_new_adventure = NEW_ADVENTURE_REGEX.is_match(&description);
    let is_new_campaign = NEW_CAMPAIGN_REGEX.is_match(&description);
    let is_online = event.is_online() || ONLINE_REGEX.is_match(&description);
    let event_series_captures = EVENT_SERIES_REGEX.captures(&description);
    let channel_captures = CHANNEL_REGEX.captures(&description);
    let category_captures = CATEGORY_REGEX.captures(&description);
    let indicated_channel_id = match channel_captures {
        Some(captures) => match captures.name("channel_id") {
            Some(id) => match id.as_str().parse::<u64>() {
                Ok(id) => Some(id),
                _ => return Err(SimpleError::new("Invalid channel id").into()),
            },
            _ => return Err(SimpleError::new("Internal error parsing channel id").into()),
        },
        _ => None,
    };
    let category_id = match category_captures {
        Some(captures) => match captures.name("category_id") {
            Some(id) => match id.as_str().parse::<u64>() {
                Ok(id) => Some(id),
                _ => {
                    eprintln!(
                        "Event {} specifies invalid category ID {}",
                        title,
                        id.as_str()
                    );
                    None
                }
            },
            _ => {
                eprintln!("Internal error parsing category ID");
                None
            }
        },
        _ => None,
    };
    let urlname = if let Some(urlname) = event.group.as_ref().map(|group| group.urlname) {
        urlname
    } else {
        eprintln!("Event {} is missing a group urlname", title,);
        return Ok(());
    };
    if indicated_channel_id.is_some() && !(is_new_adventure || is_new_campaign) {
        return Err(SimpleError::new(format!(
            "Skipping event \"{}\" since it indicates a channel to be connected with but is not \
             the start of a new series",
            title
        ))
        .into());
    }
    // Either: new adventure, new campaign, or continuation (event series)
    if !(is_new_adventure || is_new_campaign || event_series_captures.is_some()) {
        println!("Syncing task: Ignoring event \"{}\"", title);
        return Ok(());
    } else {
        println!("Syncing task: found event \"{}\"", title);
    }
    if event_series_captures.is_some()
        && (is_new_adventure || is_new_campaign || indicated_channel_id.is_some())
    {
        eprintln!(
            "Syncing task: Event \"{}\" specifies a series as well as a new adventure/campaign \
             tag, ignoring",
            title
        );
        return Ok(());
    }

    let mut tx = db_connection.begin().await?;

    // Check if this event already exists in the database
    let row = sqlx::query!(
        r#"SELECT meetup_event.id as "meetup_event_id", event.id as "event_id", event.event_series_id
        FROM meetup_event
        INNER JOIN event ON meetup_event.event_id = event.id
        WHERE meetup_event.meetup_id = $1
        FOR UPDATE"#,
        event.id).fetch_optional(&mut *tx).await?;
    let db_meetup_event_id = row.as_ref().map(|row| row.meetup_event_id);
    let db_event_id = row.as_ref().map(|row| row.event_id);
    let existing_series_id = row.as_ref().map(|row| row.event_series_id);

    // If this is part of an event series, figure out which
    let indicated_event_series_id = if let Some(event_series_captures) = event_series_captures {
        // This is the event ID of an event that belongs to this series
        let series_event_id = match event_series_captures.name("event_id") {
            Some(id) => id.as_str(),
            None => {
                eprintln!("Syncing task: error capturing event_id");
                return Ok(());
            }
        };
        // Look up that event's series ID
        let event_series_id = sqlx::query_scalar!(
            r#"SELECT event.event_series_id
            FROM event
            INNER JOIN meetup_event ON event.id = meetup_event.event_id
            WHERE meetup_event.meetup_id = $1"#,
            series_event_id
        )
        .fetch_optional(&mut *tx)
        .await?;
        if event_series_id.is_none() {
            eprintln!("Event syncing task: Meetup event {} indicates that it is part of the same event series as Meetup event {} but the latter is not in the database", event.id, series_event_id);
            return Ok(());
        }
        event_series_id
    } else {
        None
    };

    // If a channel was indicated make sure it's not managed already
    if let Some(indicated_channel_id) = indicated_channel_id {
        let is_managed_channel = sqlx::query_scalar!(
            r#"SELECT COUNT(*) > 0 as "is_managed!" FROM managed_channel WHERE discord_id = $1"#,
            indicated_channel_id as i64
        )
        .fetch_one(&mut *tx)
        .await?;
        if is_managed_channel {
            eprintln!("Event syncing task: Meetup event {} indicates a channel but that channel is already managed", event.id);
            return Ok(());
        }
    }
    // Is there already a series ID for the possibly indicated channel?
    let indicated_channel_series = if let Some(indicated_channel_id) = indicated_channel_id {
        let indicated_channel_series = sqlx::query_scalar!(
            r#"SELECT id
            FROM event_series
            WHERE discord_text_channel_id = $1"#,
            indicated_channel_id as i64
        )
        .fetch_optional(&mut *tx)
        .await?;
        indicated_channel_series
    } else {
        None
    };
    if existing_series_id.is_none() {
        // If this event has no series ID yet but also
        // doesn't indicate that it is the start of a
        // new series or belongs to an existing series, do nothing
        if !(is_new_adventure || is_new_campaign || indicated_event_series_id.is_some()) {
            println!("Syncing task: Ignoring event \"{}\"", title);
            return Ok(());
        }
        // If this event has no series ID yet, but the channel
        // it wants to be associated with does, then something is fishy
        if indicated_channel_series.is_some() {
            println!(
                "Event \"{}\" wants to be associated with a certain channel but that \
                             channel already belongs to an event series",
                title
            );
            return Ok(());
        }
    }
    // Use the existing series ID or create a new one
    let series_id = match existing_series_id {
        Some(existing_series_id) => {
            // This event was already synced before and as such already has an event series ID.

            // If this event's series ID does not match the channel's series ID, something is fishy
            if let Some(channel_series) = indicated_channel_series {
                if channel_series != existing_series_id {
                    eprintln!(
                        "Event \"{}\" wants to be associated with a certain channel \
                         but that channel already belongs to a different event series",
                        title
                    );
                    return Ok(());
                }
            }
            // If this event's series ID does not match the indicated event series ID, issue a warning
            if let Some(indicated_event_series_id) = indicated_event_series_id {
                if &existing_series_id != &indicated_event_series_id {
                    eprintln!(
                        "Warning: Event \"{}\" indicates event series {} but is \
                         already associated with event series {}.",
                        title, indicated_event_series_id, existing_series_id
                    );
                    return Ok(());
                }
            }
            existing_series_id
        }
        None => {
            // This event has not been synced before and we either create a new event series ID
            // for new campaigns/adventures or we connect it to an existing event series
            if (is_new_adventure || is_new_campaign) && indicated_event_series_id.is_none() {
                let new_series_type = if is_new_adventure {
                    "adventure"
                } else {
                    "campaign"
                };
                let new_series_id = if let Some(channel_id) = indicated_channel_id {
                    // If this event wants to be associated with a channel but that channel already
                    // has an event series ID, something is fishy
                    if indicated_channel_series.is_some() {
                        eprintln!(
                            "Event \"{}\" wants to be associated with a certain \
                             channel but that channel already belongs to a different \
                             event series",
                            title
                        );
                        return Ok(());
                    } else {
                        // The event wants to be associated with a channel and that channel is not
                        // associated to anything else yet, looking good!
                        println!(
                            "Associating event \"{}\" with Discord channel {}",
                            title, channel_id
                        );
                        let new_series_id = sqlx::query_scalar!(
                            r#"INSERT INTO event_series (discord_text_channel_id, "type") VALUES ($1, $2) RETURNING id"#,
                            channel_id as i64,
                            new_series_type
                        ).fetch_one(&mut *tx).await?;
                        new_series_id
                    }
                } else {
                    // The event does not indicate a channel to be associated
                    // with so we just create a blank event series
                    let new_series_id = sqlx::query_scalar!(
                        r#"INSERT INTO event_series ("type") VALUES ($1) RETURNING id"#,
                        new_series_type
                    )
                    .fetch_one(&mut *tx)
                    .await?;
                    new_series_id
                };
                new_series_id
            } else if let Some(indicated_event_series_id) = indicated_event_series_id {
                indicated_event_series_id
            } else {
                // Something went wrong
                eprintln!(
                    "Syncing task: internal error (event has no series id yet, but is \
                     neither a new adventure/campaign nor does it belong to a session"
                );
                return Ok(());
            }
        }
    };

    // Create or update the event and corresponding Meetup event in the database
    let db_event_id = if let Some(db_event_id) = db_event_id {
        sqlx::query_scalar!(
            r#"UPDATE event
            SET event_series_id = $1, start_time = $2, title = $3, description = $4, is_online = $5, discord_category_id = $6
            WHERE id = $7
            RETURNING id"#,
            series_id,
            event.date_time.0,
            title,
            description,
            is_online,
            category_id.map(|id| id as i64),
            db_event_id
        ).fetch_one(&mut *tx).await?
    } else {
        sqlx::query_scalar!(
            r#"INSERT INTO event (event_series_id, start_time, title, description, is_online, discord_category_id)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id"#,
            series_id,
            event.date_time.0,
            title,
            description,
            is_online,
            category_id.map(|id| id as i64)
        ).fetch_one(&mut *tx).await?
    };
    let _db_meetup_event_id = if let Some(db_meetup_event_id) = db_meetup_event_id {
        sqlx::query_scalar!(
            r#"UPDATE meetup_event
            SET url = $2, urlname = $3
            WHERE id = $1
            RETURNING id"#,
            db_meetup_event_id,
            event.event_url,
            urlname
        )
        .fetch_one(&mut *tx)
        .await?
    } else {
        sqlx::query_scalar!(
            r#"INSERT INTO meetup_event (event_id, meetup_id, url, urlname) VALUES ($1, $2, $3, $4)
            RETURNING id"#,
            db_event_id,
            event.id.0,
            event.event_url,
            urlname
        )
        .fetch_one(&mut *tx)
        .await?
    };

    // Mark event hosts
    if let Some(hosts) = event.hosts {
        for host in hosts {
            let member_id = db::get_or_create_member_for_meetup_id(&mut tx, host.id.0).await?;
            sqlx::query!(
                r#"INSERT INTO event_host (event_id, member_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"#,
                db_event_id,
                member_id.0
            ).execute(&mut *tx).await?;
        }
    }

    tx.commit().await?;

    println!("Event syncing task: Synced event \"{}\"", title);
    Ok(())
}

async fn sync_event_series(
    series_id: db::EventSeriesId,
    meetup_client: &super::newapi::AsyncClient,
    db_connection: &sqlx::PgPool,
) -> Result<(), super::Error> {
    // Get all events belonging to this event series
    let upcoming_events = db::get_upcoming_events_for_series(db_connection, series_id).await?;
    // We loop since the next event might have been deleted on Meetup.
    // So we just continue until we find one that has not been deleted or the list is exhausted.
    for next_event in upcoming_events {
        // If this event is not linked to Meetup we don't need to query any RSVPs
        let meetup_event = if let Some(meetup_event) = next_event.meetup_event {
            meetup_event
        } else {
            break;
        };
        // The first element in this vector will be the next upcoming event
        println!(
            "Syncing task: Querying RSVPs for event \"{}\"",
            next_event.title
        );
        // Query the RSVPs for that event
        let tickets = match meetup_client.get_tickets_vec(meetup_event.meetup_id).await {
            Err(super::newapi::Error::ResourceNotFound) => {
                // Remove this event from the database
                eprintln!(
                    "Event {} was deleted from Meetup, removing from database...",
                    next_event.id.0
                );
                sqlx::query!(
                    "UPDATE event SET deleted = NOW() WHERE id = $1",
                    next_event.id.0
                )
                .execute(db_connection)
                .await?;
                eprintln!("Removed event {} from database", next_event.id.0);
                continue;
            }
            Err(err) => return Err(err.into()),
            Ok(tickets) => tickets,
        };
        // Sync the RSVPs
        println!("Syncing task: Found {} RSVPs", tickets.len());
        sync_rsvps(next_event.id, tickets, db_connection).await?;
    }
    Ok(())
}

pub async fn sync_rsvps(
    event_id: db::EventId,
    tickets: Vec<super::newapi::Ticket>,
    db_connection: &sqlx::PgPool,
) -> Result<(), super::Error> {
    let rsvp_yes_user_ids: Vec<_> = tickets.iter().map(|ticket| ticket.user.id.0).collect();
    // let rsvp_no_user_ids: Vec<_> = tickets
    //     .iter()
    //     .filter_map(|rsvp| match rsvp.response {
    //         super::api::RSVPResponse::No | super::api::RSVPResponse::Waitlist => {
    //             Some(rsvp.member.id)
    //         }
    //         _ => None,
    //     })
    //     .collect();
    let mut tx = db_connection.begin().await?;
    // Convert the Meetup users IDs into member IDs
    let mut rsvp_yes_member_ids = Vec::with_capacity(rsvp_yes_user_ids.len());
    for user_id in rsvp_yes_user_ids {
        let member_id = db::get_or_create_member_for_meetup_id(&mut tx, user_id).await?;
        rsvp_yes_member_ids.push(member_id);
    }
    // Remove all users which are not attending
    sqlx::query!(
        r#"DELETE FROM event_participant WHERE event_id = $1 AND NOT (member_id = ANY($2))"#,
        event_id.0,
        &rsvp_yes_member_ids
            .iter()
            .map(|id| id.0)
            .collect::<Vec<_>>()
    )
    .execute(&mut *tx)
    .await?;
    for member_id in rsvp_yes_member_ids {
        // Mark this member as attending
        sqlx::query!(
            r#"INSERT INTO event_participant (event_id, member_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"#,
            event_id.0,
            member_id.0
        ).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}
