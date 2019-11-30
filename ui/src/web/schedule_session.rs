use super::server::HandlerResponse;
use askama::Template;
use chrono::{offset::TimeZone, Datelike, Timelike};
use chrono_tz::Europe;
use futures_util::{compat::Future01CompatExt, lock::Mutex, TryFutureExt};
use std::{collections::HashMap, sync::Arc};
use warp::Filter;

pub fn create_routes(
    redis_client: redis::Client,
    meetup_client: Arc<Mutex<Option<Arc<lib::meetup::api::AsyncClient>>>>,
    oauth2_consumer: Arc<lib::meetup::oauth2::OAuth2Consumer>,
    discord_cache_http: lib::discord::CacheAndHttp,
) -> impl warp::Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    let get_route = {
        let redis_client = redis_client.clone();
        warp::get()
            .and(warp::path!("schedule_session" / u64))
            .and(warp::path::end())
            .and_then(move |flow_id| {
                let redis_client = redis_client.clone();
                async move {
                    let redis_connection = redis_client
                        .get_async_connection()
                        .compat()
                        .err_into::<lib::meetup::Error>()
                        .await?;
                    handle_schedule_session(redis_connection, flow_id)
                        .err_into::<warp::Rejection>()
                        .await
                }
            })
    };
    #[cfg(feature = "bottest")]
    let get_route = {
        let get_test_route = warp::get()
            .and(warp::path!("schedule_session" / "test"))
            .and(warp::path::end())
            .and_then(|| {
                let local_time = chrono::Utc::now().with_timezone(&Europe::Zurich);
                let template = ScheduleSessionTemplate {
                    day: local_time.day() as u8,
                    month: local_time.month() as u8,
                    year: local_time.year() as u16,
                    hour: local_time.hour() as u8,
                    minute: local_time.minute() as u8,
                    selectable_years: &[local_time.year() as u16, local_time.year() as u16 + 1],
                    title: "Test event",
                    link: "https://meetup.com/",
                };
                futures::future::ready(
                    HandlerResponse::from_template(template)
                        .map_err(|err| warp::Rejection::from(err)),
                )
            });
        let get_test_success_route = warp::get()
            .and(warp::path!("schedule_session" / "test" / "success"))
            .and(warp::path::end())
            .and_then(|| {
                let template = ScheduleSessionSuccessTemplate {
                    title: "Test event",
                    link: "https://meetup.com/",
                    transferred_all_rsvps: Some(true),
                    closed_rsvps: true,
                };
                futures::future::ready(
                    HandlerResponse::from_template(template)
                        .map_err(|err| warp::Rejection::from(err)),
                )
            });
        get_test_route.or(get_test_success_route).or(get_route)
    };
    let post_route = {
        let redis_client = redis_client.clone();
        let meetup_client = meetup_client.clone();
        let oauth2_consumer = oauth2_consumer.clone();
        let discord_cache_http = discord_cache_http.clone();
        warp::post()
            .and(warp::path!("schedule_session" / u64))
            .and(warp::path::end())
            .and(warp::body::content_length_limit(32 * 1024))
            .and(warp::body::form())
            .and_then(move |flow_id, form_data: HashMap<String, String>| {
                let mut redis_client = redis_client.clone();
                let meetup_client = meetup_client.clone();
                let oauth2_consumer = oauth2_consumer.clone();
                let discord_cache_http = discord_cache_http.clone();
                async move {
                    handle_schedule_session_post(
                        &mut redis_client,
                        &meetup_client,
                        oauth2_consumer,
                        &discord_cache_http,
                        flow_id,
                        form_data,
                    )
                    .err_into::<warp::Rejection>()
                    .await
                }
            })
    };
    get_route.or(post_route)
}

#[derive(Template)]
#[template(path = "schedule_session.html")]
struct ScheduleSessionTemplate<'a> {
    day: u8,
    month: u8,
    year: u16,
    hour: u8,
    minute: u8,
    selectable_years: &'a [u16],
    title: &'a str,
    link: &'a str,
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
}

async fn handle_schedule_session(
    redis_connection: redis::aio::Connection,
    flow_id: u64,
) -> Result<super::server::HandlerResponse, lib::meetup::Error> {
    eprintln!("Retrieving flow...");
    let (redis_connection, flow) =
        lib::flow::ScheduleSessionFlow::retrieve(redis_connection, flow_id).await?;
    let flow = match flow {
        Some(flow) => flow,
        None => return Ok(("Link expired", "Please request a new link").into()),
    };
    eprintln!("... got it!\nRetrieving events...");
    let (_redis_connection, mut events) =
        lib::meetup::util::get_events_for_series_async(redis_connection, &flow.event_series_id)
            .await?;
    eprintln!("... got them!");
    // Sort by date
    events.sort_unstable_by_key(|event| event.time);
    if let Some(event) = events.last() {
        // Assume Swiss time
        let local_time = event.time.with_timezone(&Europe::Zurich);
        // We don't just add 7 * 24 hours, since that might break across
        // daylight saving time boundaries
        let next_event_local_datetime =
            match (local_time.date() + chrono::Duration::weeks(1)).and_time(local_time.time()) {
                Some(time) => time,
                None => local_time,
            };
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
            title: &event.name,
            link: &event.link,
        };
        Ok(HandlerResponse::from_template(template)?)
    } else {
        Ok((
            "No prior event found",
            "Cannot schedule a continuation session without an initial event",
        )
            .into())
    }
}

async fn handle_schedule_session_post(
    redis_client: &mut redis::Client,
    meetup_client: &Mutex<Option<Arc<lib::meetup::api::AsyncClient>>>,
    oauth2_consumer: Arc<lib::meetup::oauth2::OAuth2Consumer>,
    discord_cache_http: &lib::discord::CacheAndHttp,
    flow_id: u64,
    form_data: HashMap<String, String>,
) -> Result<super::server::HandlerResponse, lib::meetup::Error> {
    let redis_connection = redis_client
        .get_async_connection()
        .compat()
        .err_into::<lib::meetup::Error>()
        .await?;
    let (redis_connection, flow) =
        lib::flow::ScheduleSessionFlow::retrieve(redis_connection, flow_id).await?;
    let flow = match flow {
        Some(flow) => flow,
        None => return Ok(("Link expired", "Please request a new link").into()),
    };
    let (redis_connection, mut events) =
        lib::meetup::util::get_events_for_series_async(redis_connection, &flow.event_series_id)
            .await?;
    // Sort by date
    events.sort_unstable_by_key(|event| event.time);
    if let Some(event) = events.last() {
        // Check that the form contains all necessary data
        let transfer_rsvps = form_data
            .get("transfer_rsvps")
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
                return Ok((
                    "Invalid data",
                    "Seems like the submitted data is incomplete",
                )
                    .into())
            }
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
                                _ => return Ok((
                                    "Invalid data",
                                    "Seems like the specified time is ambiguous or non-existent",
                                )
                                    .into()),
                            }
                        }
                        _ => {
                            return Ok(
                                ("Invalid data", "Seems like the specified time is invalid").into()
                            )
                        }
                    },
                    _ => {
                        return Ok(
                            ("Invalid data", "Seems like the specified date is invalid").into()
                        )
                    }
                }
            }
            _ => {
                return Ok((
                    "Invalid data",
                    "Seems like the submitted data has an invalid format",
                )
                    .into())
            }
        };
        // Convert time to UTC
        let date_time = date_time.with_timezone(&chrono::Utc);
        let new_event_hook = Box::new(|new_event: lib::meetup::api::NewEvent| {
            lib::flow::ScheduleSessionFlow::new_event_hook(new_event, date_time, &event.id)
        });
        let meetup_client = match *meetup_client.lock().await {
            Some(ref meetup_client) => meetup_client.clone(),
            None => return Ok(("Meetup API unavailable", "Please try again later").into()),
        };
        let new_event = lib::meetup::util::clone_event(
            &event.urlname,
            &event.id,
            &meetup_client,
            Some(new_event_hook),
        )
        .await?;
        // Close the RSVPs, ignoring errors
        let rsvps_are_closed =
            if let Err(err) = meetup_client.close_rsvps(&event.urlname, &event.id).await {
                eprintln!(
                    "RSVPs for event {} could not be closed:\n{:#?}",
                    &event.id, err
                );
                false
            } else {
                true
            };
        // Delete the flow, ignoring errors
        let _ = flow.delete(redis_connection);
        let transferred_all_rsvps = if transfer_rsvps {
            // Try to transfer the RSVPs to the new event
            if let Err(_) = lib::meetup::util::clone_rsvps(
                &event.urlname,
                &event.id,
                &new_event.id,
                redis_client,
                &meetup_client,
                oauth2_consumer.as_ref(),
            )
            .await
            {
                Some(false)
            } else {
                Some(true)
            }
        } else {
            None
        };
        // Announce the new session in the Discord channel
        let message = if transfer_rsvps {
            format!(
                "Your adventure continues @here: {link}. \
                 Slay the dragon, save the prince, get the treasure, \
                 or whatever shenanigans you like to get into.",
                link = &new_event.link
            )
        } else {
            format!(
                "Your adventure continues @here: {link}. \
                 Slay the dragon, save the prince, get the treasure, \
                 or whatever shenanigans you like to get into.\n\
                 <@{publisher_id}>, please announce this session for new players to join.",
                link = &new_event.link,
                publisher_id = lib::discord::sync::ids::PUBLISHER_ID.0
            )
        };
        // TODO: remove this superfluous Redis connection once the new async API is available
        if let Ok(redis_connection) = redis_client.get_async_connection().compat().await {
            let _ = lib::discord::util::say_in_event_series_channel(
                &new_event.id,
                &message,
                redis_connection,
                discord_cache_http,
            );
        }
        let template = ScheduleSessionSuccessTemplate {
            title: &new_event.name,
            link: &new_event.link,
            transferred_all_rsvps: transferred_all_rsvps,
            closed_rsvps: rsvps_are_closed,
        };
        Ok(HandlerResponse::from_template(template)?)
    } else {
        Ok((
            "No prior event found",
            "Cannot schedule a continuation session without an initial event",
        )
            .into())
    }
}
