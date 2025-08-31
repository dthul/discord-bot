use std::{collections::HashMap, sync::Arc};

use askama::Template;
use axum::{
    extract::{DefaultBodyLimit, Extension, Form, Path},
    handler::Handler,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use chrono::{offset::TimeZone, Datelike, NaiveDateTime, Timelike};
use chrono_tz::Europe;
use lib::db;
use serenity::all::Mentionable;

use super::{server::State, MessageTemplate, WebError};

pub fn create_routes() -> Router {
    let routes = Router::new().route(
        "/schedule_session/:flow_id",
        get(schedule_session_handler)
            .post(schedule_session_post_handler.layer(DefaultBodyLimit::max(32768))),
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
    let mut redis_connection = state.redis_client.get_multiplexed_async_connection().await?;
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
            let mut next_event_local_datetime = local_time + chrono::Days::new(7);
            // If the proposed next event time is in the past, propose a time in the future instead
            let now = chrono::Utc::now().with_timezone(&Europe::Zurich);
            if next_event_local_datetime < now {
                next_event_local_datetime = next_event_local_datetime
                    .timezone()
                    .from_local_datetime(&NaiveDateTime::new(
                        now.date_naive() + chrono::Days::new(1),
                        local_time.time(),
                    ))
                    .earliest()
                    .expect("DateTime for tomorrow is valid");
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
                    .map(|meetup_event| meetup_event.url.as_str())
                    .or_else(|| event.swissrpg_event.as_ref().map(|swissrpg_event| swissrpg_event.url.as_str())),
            };
            Ok(template.into_response())
        }
    }
}

#[tracing::instrument(skip(state, form_data), fields(flow_id = %flow_id))]
async fn schedule_session_post_handler(
    Extension(state): Extension<Arc<State>>,
    Path(flow_id): Path<u64>,
    Form(form_data): Form<HashMap<String, String>>,
) -> Result<Response, WebError> {
    let mut redis_connection = state.redis_client.get_multiplexed_async_connection().await?;
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
    let meetup_client_guard = state.async_meetup_client.lock().await;
    let meetup_client = meetup_client_guard.as_deref();
    
    // Capture the event series ID before moving the flow
    let event_series_id = flow.event_series_id;
    
    // Use the new unified scheduling approach
    let schedule_result = flow.schedule(
        state.pool.clone(),
        redis_connection,
        meetup_client,
        Some(state.swissrpg_client.clone()),
        date_time,
        is_open_game,
    ).await;
    
    let (new_event_title, new_event_url, is_meetup) = match &schedule_result {
        Ok(lib::flow::ScheduleSessionResult::Meetup(meetup_event)) => {
            (meetup_event.title.clone().unwrap_or_else(|| "No title".to_string()), meetup_event.event_url.clone(), true)
        },
        Ok(lib::flow::ScheduleSessionResult::SwissRPG(swissrpg_event)) => {
            (swissrpg_event.title.clone(), swissrpg_event.public_url.clone(), false)
        },
        Err(err) => {
            let template: MessageTemplate = ("Scheduling failed", format!("Error: {}", err)).into();
            return Ok(template.into_response());
        }
    };
    
    // Only close RSVPs for Meetup events for now
    let rsvps_are_closed = if is_meetup {
        if let Some(meetup_client) = meetup_client {
            // For Meetup events, we need to extract the event ID from the URL or result
            // This is a simplified approach - in practice you'd want to get the ID from the result
            match &schedule_result {
                Ok(lib::flow::ScheduleSessionResult::Meetup(meetup_event)) => {
                    if let Err(err) = meetup_client.close_rsvps(meetup_event.id.0.clone()).await {
                        eprintln!("RSVPs could not be closed: {:#?}", err);
                        false
                    } else {
                        true
                    }
                }
                _ => false,
            }
        } else {
            false
        }
    } else {
        false // SwissRPG events handle their own RSVP state
    };

    // Remove any possibly existing channel snoozes
    {
        let mut tx = state.pool.begin().await?;
        if let Ok(Some(channel_id)) =
            lib::get_series_text_channel(event_series_id, &mut tx).await
        {
            sqlx::query!(r#"UPDATE event_series_text_channel SET snooze_until = NULL WHERE discord_id = $1"#, channel_id.get() as i64).execute(&mut *tx).await.ok();
            tx.commit().await.ok();
        }
    }
    
    // Get the latest event to find the Discord channel for announcements
    let latest_event = lib::db::get_last_event_in_series(&state.pool, event_series_id).await?;
    
    if let Some(latest_event) = latest_event {
        // Announce the new session in the Discord channel
        let channel_roles =
            lib::get_event_series_roles(event_series_id, &mut state.pool.begin().await?).await?;
        let message = if let Some(channel_roles) = channel_roles {
            format!(
                "Your adventure continues here, heroes of {channel_role_mention}: {link}. Slay the \
                 dragon, save the prince, get the treasure, or whatever shenanigans you like to \
                 get into.",
                link = &new_event_url,
                channel_role_mention = channel_roles.user.mention()
            )
        } else {
            format!(
                "Your adventure continues @here: {link}. Slay the dragon, save the prince, get \
                 the treasure, or whatever shenanigans you like to get into.",
                link = &new_event_url
            )
        };
        if let Err(err) = lib::discord::util::say_in_event_channel(
            latest_event.id,
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
                "{organiser_mention}, a new session has been scheduled:\n{link}.\nPlease announce \
                 this session for new players to join. Don't forget to **open RSVPs** when you do \
                 that.",
                organiser_mention = lib::discord::sync::ids::ORGANISER_ID.mention(),
                link = &new_event_url,
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
    }
    
    let template = ScheduleSessionSuccessTemplate {
        title: &new_event_title,
        link: &new_event_url,
        transferred_all_rsvps: None, // Not implementing RSVP transfer for now
        closed_rsvps: rsvps_are_closed,
    };
    Ok(template.into_response())
}
