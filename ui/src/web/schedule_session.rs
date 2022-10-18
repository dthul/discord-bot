use std::{collections::HashMap, sync::Arc};

use askama::Template;
use axum::{
    extract::{ContentLengthLimit, Extension, Form, Path},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use chrono::{offset::TimeZone, Datelike, Timelike};
use chrono_tz::Europe;
use lib::{db, DefaultStr};

use super::{server::State, MessageTemplate, WebError};

pub fn create_routes() -> Router {
    let routes = Router::new().route(
        "/schedule_session/:flow_id",
        get(schedule_session_handler).post(schedule_session_post_handler),
    );
    // The following routes are just to be able to take a look at the scheduling
    // and success templates without using an actual flow
    #[cfg(feature = "bottest")]
    let routes = routes
        .route(
            "/schedule_session/test",
            get(|| {
                let local_time = chrono::Utc::now().with_timezone(&Europe::Zurich);
                let template = ScheduleSessionTemplate {
                    day: local_time.day() as u8,
                    month: local_time.month() as u8,
                    year: local_time.year() as u16,
                    hour: local_time.hour() as u8,
                    minute: local_time.minute() as u8,
                    selectable_years: &[local_time.year() as u16, local_time.year() as u16 + 1],
                    duration: 150,
                    title: "Test event",
                    link: Some("https://meetup.com/"),
                };
                futures::future::ready(template.into_response())
            }),
        )
        .route(
            "/schedule_session/test/success",
            get(|| {
                let template = ScheduleSessionSuccessTemplate {
                    title: "Test event",
                    link: "https://meetup.com/",
                    transferred_all_rsvps: Some(true),
                    closed_rsvps: true,
                };
                futures::future::ready(template.into_response())
            }),
        );
    routes
}

#[derive(Template)]
#[template(path = "schedule_session.html")]
struct ScheduleSessionTemplate<'a> {
    day: u8, // In Europe::Zurich timezone
    month: u8,
    year: u16,
    hour: u8,
    minute: u8,
    selectable_years: &'a [u16],
    duration: u16, // In minutes
    title: &'a str,
    link: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "schedule_session_success.html")]
struct ScheduleSessionSuccessTemplate<'a> {
    title: &'a str,
    link: &'a str,
    transferred_all_rsvps: Option<bool>,
    closed_rsvps: bool,
}

pub mod filters {
    pub fn isequal<T: num_traits::PrimInt>(num: &T, val: &T) -> Result<bool, askama::Error> {
        Ok(num == val)
    }

    pub fn format_minutes_to_hhmm(minutes: &u16) -> Result<String, askama::Error> {
        Ok(format!("{}:{:02}", minutes / 60, minutes % 60))
    }
}

async fn schedule_session_handler(
    Extension(state): Extension<Arc<State>>,
    Path(flow_id): Path<u64>,
) -> Result<Response, WebError> {
    let mut redis_connection = state.redis_client.get_async_connection().await?;
    eprintln!("Retrieving flow...");
    let flow = lib::flow::ScheduleSessionFlow::retrieve(&mut redis_connection, flow_id).await?;
    let flow = match flow {
        Some(flow) => flow,
        None => {
            let template: MessageTemplate = ("Link expired", "Please request a new link").into();
            return Ok(template.into_response());
        }
    };
    eprintln!("... got it!\nRetrieving last event...");
    let event = db::get_last_event_in_series(&state.pool, flow.event_series_id).await?;
    eprintln!("... got it!");
    match event {
        None => {
            let template: MessageTemplate = (
                "No prior event found",
                "Cannot schedule a continuation session without an initial event",
            )
                .into();
            Ok(template.into_response())
        }
        Some(event) => {
            // Assume Swiss time
            let local_time = event.time.with_timezone(&Europe::Zurich);
            // We don't just add 7 * 24 hours, since that might break across
            // daylight saving time boundaries
            let mut next_event_local_datetime = match (local_time.date()
                + chrono::Duration::weeks(1))
            .and_time(local_time.time())
            {
                Some(time) => time,
                None => local_time,
            };
            // If the proposed next event time is in the past, propose a time in the future instead
            let now = chrono::Utc::now().with_timezone(&Europe::Zurich);
            if next_event_local_datetime < now {
                next_event_local_datetime = now
                    .with_timezone(&Europe::Zurich)
                    .date()
                    .and_time(local_time.time())
                    .unwrap_or(now + chrono::Duration::days(1));
            }
            let template = ScheduleSessionTemplate {
                day: next_event_local_datetime.day() as u8,
                month: next_event_local_datetime.month() as u8,
                year: next_event_local_datetime.year() as u16,
                hour: next_event_local_datetime.hour() as u8,
                minute: next_event_local_datetime.minute() as u8,
                selectable_years: &[
                    next_event_local_datetime.year() as u16,
                    next_event_local_datetime.year() as u16 + 1,
                ],
                duration: 4 * 60,
                title: &event.title,
                link: event
                    .meetup_event
                    .as_ref()
                    .map(|meetup_event| meetup_event.url.as_str()),
            };
            Ok(template.into_response())
        }
    }
}

async fn schedule_session_post_handler(
    _: ContentLengthLimit<(), 32768>,
    Extension(state): Extension<Arc<State>>,
    Path(flow_id): Path<u64>,
    Form(form_data): Form<HashMap<String, String>>,
) -> Result<Response, WebError> {
    let mut redis_connection = state.redis_client.get_async_connection().await?;
    let flow = lib::flow::ScheduleSessionFlow::retrieve(&mut redis_connection, flow_id).await?;
    let flow = match flow {
        Some(flow) => flow,
        None => {
            let template: MessageTemplate = ("Link expired", "Please request a new link").into();
            return Ok(template.into_response());
        }
    };
    // Check that the form contains all necessary data
    let transfer_rsvps = form_data
        .get("transfer_rsvps")
        .map(|value| value == "yes")
        .unwrap_or(false);
    let is_open_game = form_data
        .get("open_game")
        .map(|value| value == "yes")
        .unwrap_or(false);
    let (year, month, day, hour, minute) = match (
        form_data.get("year"),
        form_data.get("month"),
        form_data.get("day"),
        form_data.get("hour"),
        form_data.get("minute"),
    ) {
        (Some(year), Some(month), Some(day), Some(hour), Some(minute)) => {
            (year, month, day, hour, minute)
        }
        _ => {
            let template: MessageTemplate = (
                "Invalid data",
                "Seems like the submitted data is incomplete",
            )
                .into();
            return Ok(template.into_response());
        }
    };
    let duration = match form_data.get("duration") {
        None => 4 * 60,
        Some(duration) => match duration.parse::<u16>() {
            Err(_) => 4 * 60,
            Ok(duration) => duration.min(12 * 60),
        },
    };
    // Try to convert the supplied data to a DateTime
    let date_time = match (
        year.parse::<i32>(),
        month.parse::<u32>(),
        day.parse::<u32>(),
        hour.parse::<u32>(),
        minute.parse::<u32>(),
    ) {
        (Ok(year), Ok(month), Ok(day), Ok(hour), Ok(minute)) => {
            match chrono::NaiveDate::from_ymd_opt(year, month, day) {
                Some(date) => match date.and_hms_opt(hour, minute, 0) {
                    Some(naive_date_time) => {
                        match Europe::Zurich.from_local_datetime(&naive_date_time) {
                            chrono::LocalResult::Single(date_time) => date_time,
                            _ => {
                                let template: MessageTemplate = (
                                    "Invalid data",
                                    "Seems like the specified time is ambiguous or non-existent",
                                )
                                    .into();
                                return Ok(template.into_response());
                            }
                        }
                    }
                    _ => {
                        let template: MessageTemplate =
                            ("Invalid data", "Seems like the specified time is invalid").into();
                        return Ok(template.into_response());
                    }
                },
                _ => {
                    let template: MessageTemplate =
                        ("Invalid data", "Seems like the specified date is invalid").into();
                    return Ok(template.into_response());
                }
            }
        }
        _ => {
            let template: MessageTemplate = (
                "Invalid data",
                "Seems like the submitted data has an invalid format",
            )
                .into();
            return Ok(template.into_response());
        }
    };
    // Convert time to UTC
    let date_time = date_time.with_timezone(&chrono::Utc);
    let meetup_client = match *(state.async_meetup_client).lock().await {
        Some(ref meetup_client) => meetup_client.clone(),
        None => {
            let template: MessageTemplate =
                ("Meetup API unavailable", "Please try again later").into();
            return Ok(template.into_response());
        }
    };
    // We go from the latest event to the oldest event until we find one
    // that has not been deleted to use as a template
    let event_series_id = flow.event_series_id.clone();
    let events = db::get_events_for_series(&state.pool, event_series_id).await?;
    for event in events {
        let meetup_event = if let Some(meetup_event) = event.meetup_event {
            meetup_event
        } else {
            continue;
        };
        let new_event_hook = Box::new(|mut new_event: lib::meetup::newapi::NewEvent| {
            new_event.duration = Some(chrono::Duration::minutes(duration as i64).into());
            new_event.publish_status =
                Some(lib::meetup::newapi::create_event_mutation::PublishStatus::PUBLISHED);
            let result = lib::flow::ScheduleSessionFlow::new_event_hook(
                new_event,
                date_time,
                &meetup_event.meetup_id,
                is_open_game,
            );
            if let Ok(event_input) = &result {
                println!(
                    "Trying to create a Meetup event with the following details:\n{:#?}",
                    event_input
                );
            }
            result
        }) as _;
        let new_event = match lib::meetup::util::clone_event(
            &meetup_event.urlname,
            &meetup_event.meetup_id,
            &meetup_client,
            Some(new_event_hook),
        )
        .await
        {
            Err(lib::meetup::Error::NewAPIError(lib::meetup::newapi::Error::ResourceNotFound)) => {
                // Event was deleted, try the next one
                continue;
            }
            Err(err) => return Err(err.into()),
            Ok(new_event) => new_event,
        };
        // Delete the flow, ignoring errors
        if let Err(err) = flow.delete(&mut redis_connection).await {
            eprintln!(
                "Encountered an error when trying to delete a schedule session flow:\n{:#?}",
                err
            );
        }
        let transferred_all_rsvps = if transfer_rsvps {
            // // Try to transfer the RSVPs to the new event
            // if let Err(_) = lib::meetup::util::clone_rsvps(
            //     &event.urlname,
            //     &event.id,
            //     &new_event.id,
            //     redis_connection,
            //     &meetup_client,
            //     oauth2_consumer.as_ref(),
            // )
            // .await
            // {
            //     Some(false)
            // } else {
            //     Some(true)
            // }
            Some(false)
        } else {
            None
        };
        // // Close the RSVPs, ignoring errors
        let rsvps_are_closed =
            if let Err(err) = meetup_client.close_rsvps(new_event.id.0.clone()).await {
                eprintln!(
                    "RSVPs for event {} could not be closed:\n{:#?}",
                    &new_event.id, err
                );
                false
            } else {
                true
            };
        // Remove any possibly existing channel snoozes
        {
            let mut tx = state.pool.begin().await?;
            if let Ok(Some(channel_id)) =
                lib::get_series_text_channel(event_series_id, &mut tx).await
            {
                sqlx::query!(r#"UPDATE event_series_text_channel SET snooze_until = NULL WHERE discord_id = $1"#, channel_id.0 as i64).execute(&mut tx).await.ok();
                tx.commit().await.ok();
            }
        }
        // Announce the new session in the Discord channel
        let channel_roles =
            lib::get_event_series_roles(event_series_id, &mut state.pool.begin().await?).await?;
        let message = if let Some(channel_roles) = channel_roles {
            format!(
                "Your adventure continues here, heroes of <@&{channel_role_id}>: {link}. Slay the \
                 dragon, save the prince, get the treasure, or whatever shenanigans you like to \
                 get into.",
                link = &new_event.short_url,
                channel_role_id = channel_roles.user
            )
        } else {
            format!(
                "Your adventure continues @here: {link}. Slay the dragon, save the prince, get \
                 the treasure, or whatever shenanigans you like to get into.",
                link = &new_event.short_url
            )
        };
        if let Err(err) = lib::discord::util::say_in_event_channel(
            event.id,
            &message,
            &state.pool,
            &state.discord_cache_http,
        )
        .await
        {
            eprintln!(
                "Encountered an error when trying to announce the new session in the \
                 channel:\n{:#?}",
                err
            );
        }
        // If RSVPs were not transferred, announce the new session in the bot alerts channel
        if is_open_game {
            let message = format!(
                "<@&{organiser_id}>, a new session has been scheduled:\n{link}.\nPlease announce \
                 this session for new players to join. Don't forget to **open RSVPs** when you do \
                 that.",
                organiser_id = lib::discord::sync::ids::ORGANISER_ID.0,
                link = &new_event.event_url,
            );
            if let Err(err) =
                lib::discord::util::say_in_bot_alerts_channel(&message, &state.discord_cache_http)
                    .await
            {
                eprintln!(
                    "Encountered an error when trying to announce a new session in the bot alerts \
                     channel:\n{:#?}",
                    err
                );
            }
        }
        let template = ScheduleSessionSuccessTemplate {
            title: new_event.title.unwrap_or_str("No title"),
            link: &new_event.event_url,
            transferred_all_rsvps: transferred_all_rsvps,
            closed_rsvps: rsvps_are_closed,
        };
        return Ok(template.into_response());
    }
    let template: MessageTemplate = (
        "No prior event found",
        "Cannot schedule a continuation session without an initial event",
    )
        .into();
    Ok(template.into_response())
}
