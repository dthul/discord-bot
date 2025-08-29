use chrono::Utc;
use std::{num::NonZeroU64, sync::Arc};

use crate::{db, swissrpg::client::SwissRPGClient};

use super::schema::{Event, Session};

pub async fn sync_task(
    swissrpg_client: Arc<SwissRPGClient>,
    db_connection: &sqlx::PgPool,
) -> Result<crate::free_spots::EventCollector, crate::BoxedError> {
    let upcoming_event_series = swissrpg_client.get_events().await?;
    let mut event_collector = crate::free_spots::EventCollector::new();

    // Filter for upcoming events only
    let now = Utc::now();

    for event_series in upcoming_event_series {
        if let Some(event) = &event_series.current_session {
            if event.start > now {
                // Add to EventCollector for free spots tracking
                let common_event = crate::common_event::CommonEventDetails::from((&event_series, event));
                event_collector.add_event(common_event);
                
                match sync_event(&event_series, event, db_connection).await {
                    Err(err) => eprintln!("Event sync failed: {}", err),
                    _ => (),
                }
            }
        }

        // Also process upcoming events
        for event in &event_series.upcoming_sessions {
            if event.start > now {
                // Add to EventCollector for free spots tracking
                let common_event = crate::common_event::CommonEventDetails::from((&event_series, event));
                event_collector.add_event(common_event);
                
                match sync_event(&event_series, event, db_connection).await {
                    Err(err) => eprintln!("Event sync failed: {}", err),
                    _ => (),
                }
            }
        }
    }
    Ok(event_collector)
}

pub async fn sync_event(
    event_series: &Event,
    event: &Session,
    db_connection: &sqlx::PgPool,
) -> Result<(), crate::BoxedError> {
    let mut tx = db_connection.begin().await?;

    // Check if this event already exists in the database
    let row = sqlx::query!(
        r#"SELECT swissrpg_event.id as "swissrpg_event_id", event.id as "event_id", event.event_series_id
        FROM swissrpg_event
        INNER JOIN event ON swissrpg_event.event_id = event.id
        WHERE swissrpg_event.swissrpg_id = $1
        FOR UPDATE"#,
        event.uuid
    )
    .fetch_optional(&mut *tx)
    .await?;

    let db_swissrpg_event_id = row.as_ref().map(|row| row.swissrpg_event_id);
    let db_event_id = row.as_ref().map(|row| row.event_id);
    let existing_series_id = row.as_ref().map(|row| row.event_series_id);

    // Check if there is an event series tied to the legacy (Meetup) event ID
    let legacy_event_series_id = if let Some(legacy_series_event_id) = event_series.legacy_id {
        let event_series_id = sqlx::query_scalar!(
            r#"SELECT event.event_series_id
            FROM event
            INNER JOIN meetup_event ON event.id = meetup_event.event_id
            WHERE meetup_event.meetup_id = $1"#,
            legacy_series_event_id.to_string()
        )
        .fetch_optional(&mut *tx)
        .await?;

        if event_series_id.is_none() {
            eprintln!("Event syncing task: SwissRPG event {} indicates that it is part of the same event series as Meetup event {} but the latter is not in the database", event.uuid, legacy_series_event_id);
            return Ok(());
        }
        event_series_id
    } else {
        None
    };

    // Series ID logic (similar to meetup sync)
    let series_id = match existing_series_id {
        Some(existing_series_id) => {
            if let Some(indicated_event_series_id) = legacy_event_series_id {
                if &existing_series_id != &indicated_event_series_id {
                    eprintln!(
                        "Warning: Event \"{}\" indicates legacy event series {} but is \
                         already associated with event series {}.",
                        event_series.title, indicated_event_series_id, existing_series_id
                    );
                    return Ok(());
                }
            }
            existing_series_id
        }
        None => {
            match legacy_event_series_id {
                Some(indicated_event_series_id) => indicated_event_series_id,
                None => {
                    // TODO
                    let new_series_type = "adventure";
                    sqlx::query_scalar!(
                        r#"INSERT INTO event_series ("type") VALUES ($1) RETURNING id"#,
                        new_series_type
                    )
                    .fetch_one(&mut *tx)
                    .await?
                }
            }
        }
    };

    // Create or update the event and corresponding SwissRPG event in the database
    let db_event_id = if let Some(db_event_id) = db_event_id {
        sqlx::query_scalar!(
            r#"UPDATE event
            SET event_series_id = $1, start_time = $2, title = $3, description = $4, is_online = $5, discord_category_id = $6
            WHERE id = $7
            RETURNING id"#,
            series_id,
            event.start,
            event_series.title,
            event_series.description,
            false, // TODO: is_online
            None as Option<i64>, // TODO: category_id
            db_event_id
        ).fetch_one(&mut *tx).await?
    } else {
        sqlx::query_scalar!(
            r#"INSERT INTO event (event_series_id, start_time, title, description, is_online, discord_category_id)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id"#,
            series_id,
            event.start,
            event_series.title,
            event_series.description,
            false, // TODO: is_online
            None as Option<i64>, // TODO: category_id
        ).fetch_one(&mut *tx).await?
    };

    let _db_swissrpg_event_id = if let Some(db_swissrpg_event_id) = db_swissrpg_event_id {
        // TODO: why is this update query done? Shouldn't it just return db_swissrpg_event_id directly?
        // sqlx::query_scalar!(
        //     r#"UPDATE swissrpg_event
        //     SET swissrpg_id = $2
        //     WHERE id = $1
        //     RETURNING id"#,
        //     db_wawa_event_id,
        //     event.uuid
        // )
        // .fetch_one(&mut *tx)
        // .await?
        db_swissrpg_event_id
    } else {
        sqlx::query_scalar!(
            r#"INSERT INTO swissrpg_event (event_id, swissrpg_id, url) VALUES ($1, $2, $3)
            RETURNING id"#,
            db_event_id,
            event.uuid,
            event_series.public_url
        )
        .fetch_one(&mut *tx)
        .await?
    };

    // Mark event hosts (organisers)
    // TODO: move this up a level to only run once per event series
    for organiser in &event_series.organisers {
        let Ok(organiser_discord_id) = organiser.discord_id.parse::<NonZeroU64>().map(Into::into)
        else {
            eprintln!(
                "Invalid organiser Discord ID: {} ({})",
                organiser.discord_id, organiser.username
            );
            continue;
        };
        let member_id =
            db::get_or_create_member_for_discord_id(&mut tx, organiser_discord_id).await?;
        sqlx::query!(
            r#"INSERT INTO event_host (event_id, member_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"#,
            db_event_id,
            member_id.0
        ).execute(&mut *tx).await?;
    }

    // Mark event participants
    let mut attendee_member_ids = Vec::with_capacity(event.attendees.len());
    for participant in &event.attendees {
        let Ok(participant_discord_id) =
            participant.discord_id.parse::<NonZeroU64>().map(Into::into)
        else {
            eprintln!(
                "Invalid participant Discord ID: {} ({})",
                participant.discord_id, participant.username
            );
            continue;
        };
        let member_id =
            db::get_or_create_member_for_discord_id(&mut tx, participant_discord_id).await?;
        attendee_member_ids.push(member_id);
    }

    // Remove all users which are not attending
    sqlx::query!(
        r#"DELETE FROM event_participant WHERE event_id = $1 AND NOT (member_id = ANY($2))"#,
        db_event_id,
        &attendee_member_ids
            .iter()
            .map(|id| id.0)
            .collect::<Vec<_>>()
    )
    .execute(&mut *tx)
    .await?;

    for member_id in attendee_member_ids {
        // Mark this member as attending
        sqlx::query!(
            r#"INSERT INTO event_participant (event_id, member_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"#,
            db_event_id,
            member_id.0
        ).execute(&mut *tx).await?;
    }

    tx.commit().await?;

    println!(
        "Event syncing task: Synced event \"{}\" ({})",
        event_series.title,
        event.start.format("%Y-%m-%d %H:%M")
    );
    Ok(())
}
