use futures_util::compat::Future01CompatExt;
use redis::Commands;
use std::{
    future::Future,
    io::{self, Write},
};

async fn try_with_token_refresh<
    T,
    Ret: Future<Output = Result<T, super::api::Error>>,
    F: Fn(super::api::AsyncClient) -> Ret,
>(
    f: F,
    user_id: u64,
    redis_client: &mut redis::Client,
    oauth2_consumer: &super::oauth2::OAuth2Consumer,
) -> Result<T, super::Error> {
    let redis_connection = redis_client.get_async_connection().compat().await?;
    // Look up the Meetup access token for this user
    println!("Looking up the oauth access token");
    io::stdout().flush().unwrap();
    let redis_meetup_user_oauth_tokens_key = format!("meetup_user:{}:oauth2_tokens", user_id);
    let (_redis_connection, access_token): (_, Option<String>) = redis::cmd("HGET")
        .arg(&redis_meetup_user_oauth_tokens_key)
        .arg("access_token")
        .query_async(redis_connection)
        .compat()
        .await?;
    let access_token = match access_token {
        Some(access_token) => access_token,
        None => {
            // There is no access token: try to obtain a new one
            println!("No access token, calling oauth2_consumer.refresh_oauth_tokens");
            io::stdout().flush().unwrap();
            let (_, access_token) = oauth2_consumer
                .refresh_oauth_tokens(super::oauth2::TokenType::User(user_id), redis_client)
                .await?;
            println!("Got an access token!");
            io::stdout().flush().unwrap();
            access_token.secret().clone()
        }
    };
    // Run the provided function
    let meetup_api_user_client = super::api::AsyncClient::new(&access_token);
    println!("Running the provided funtion");
    io::stdout().flush().unwrap();
    match f(meetup_api_user_client).await {
        Err(err) => {
            println!("Got an error");
            io::stdout().flush().unwrap();
            if let super::api::Error::AuthenticationFailure = err {
                // The request seems to have failed due to invalid credentials.
                // Try to obtain a new access token and re-run the provided function.
                println!("Calling oauth2_consumer.refresh_oauth_tokens");
                io::stdout().flush().unwrap();
                let (_, access_token) = oauth2_consumer
                    .refresh_oauth_tokens(super::oauth2::TokenType::User(user_id), redis_client)
                    .await?;
                println!("Got an access token!");
                io::stdout().flush().unwrap();
                // Re-run the provided function one more time
                let meetup_api_user_client = super::api::AsyncClient::new(access_token.secret());
                f(meetup_api_user_client).await.map_err(Into::into)
            } else {
                eprintln!("Meetup API error: {:#?}", err);
                io::stdout().flush().unwrap();
                return Err(err.into());
            }
        }
        Ok(t) => {
            println!("Everything is fine!");
            io::stdout().flush().unwrap();
            Ok(t)
        }
    }
}

pub async fn rsvp_user_to_event(
    user_id: u64,
    urlname: &str,
    event_id: &str,
    redis_client: &mut redis::Client,
    oauth2_consumer: &super::oauth2::OAuth2Consumer,
) -> Result<super::api::RSVP, super::Error> {
    let rsvp_fun = |async_meetup_user_client: super::api::AsyncClient| {
        async move { async_meetup_user_client.rsvp(urlname, event_id, true).await }
    };
    try_with_token_refresh(rsvp_fun, user_id, redis_client, oauth2_consumer).await
}

pub async fn clone_event<'a>(
    urlname: &'a str,
    event_id: &'a str,
    meetup_client: &'a super::api::AsyncClient,
    hook: Option<
        Box<dyn (FnOnce(super::api::NewEvent) -> Result<super::api::NewEvent, super::Error>) + 'a>,
    >,
) -> Result<super::api::Event, super::Error> {
    let event = meetup_client.get_event(urlname, event_id).await?;
    let event = match event {
        Some(event) => event,
        None => {
            return Err(simple_error::SimpleError::new(format!(
                "Specified event ({}/{}) was not found",
                urlname, event_id
            ))
            .into())
        }
    };
    let new_event = super::api::NewEvent {
        description: event.simple_html_description.unwrap_or(event.description),
        duration_ms: event.duration_ms,
        featured_photo_id: event.featured_photo.map(|p| p.id),
        hosts: event.event_hosts.iter().map(|host| host.id).collect(),
        how_to_find_us: event.how_to_find_us,
        name: event.name,
        rsvp_limit: event.rsvp_limit,
        guest_limit: event.rsvp_rules.map(|r| r.guest_limit),
        time: event.time,
        venue_id: event
            .venue
            .map(|v| v.id)
            .ok_or(simple_error::SimpleError::new(
                "Cannot clone an event that doesn't have a venue",
            ))?,
    };
    // If there is a hook specified, let it modify the new event before
    // publishing it to Meetup
    let new_event = match hook {
        Some(hook) => hook(new_event)?,
        None => new_event,
    };
    // Post the event on Meetup
    let new_event = meetup_client.create_event(urlname, new_event).await?;
    return Ok(new_event);
}

#[derive(Debug)]
pub struct CloneRSVPResult {
    pub cloned_rsvps: Vec<super::api::RSVP>,
    pub num_success: u16,
    pub num_failure: u16,
    pub latest_error: Option<super::Error>,
}

pub async fn clone_rsvps(
    urlname: &str,
    src_event_id: &str,
    dst_event_id: &str,
    redis_client: &mut redis::Client,
    meetup_client: &super::api::AsyncClient,
    oauth2_consumer: &super::oauth2::OAuth2Consumer,
) -> Result<CloneRSVPResult, super::Error> {
    // First, query the source event's RSVPs and filter them by "yes" responses
    let rsvps: Vec<_> = meetup_client
        .get_rsvps(urlname, src_event_id)
        .await?
        .into_iter()
        .filter(|rsvp| rsvp.response == super::api::RSVPResponse::Yes)
        .collect();
    // Now, try to RSVP each user to the destination event
    let mut result = CloneRSVPResult {
        cloned_rsvps: Vec::with_capacity(rsvps.len()),
        num_success: 0,
        num_failure: 0,
        latest_error: None,
    };
    for rsvp in &rsvps {
        match rsvp_user_to_event(
            rsvp.member.id,
            urlname,
            dst_event_id,
            redis_client,
            oauth2_consumer,
        )
        .await
        {
            Ok(rsvp) => {
                result.cloned_rsvps.push(rsvp);
                result.num_success += 1;
            }
            Err(err) => {
                eprintln!(
                    "Could not RSVP user {} to event {}:\n{:#?}",
                    rsvp.member.id, dst_event_id, err
                );
                result.latest_error = Some(err);
                result.num_failure += 1;
            }
        }
    }
    Ok(result)
}

// TODO: move to redis module?
pub struct Event {
    pub id: String,
    pub name: String,
    pub time: chrono::DateTime<chrono::Utc>,
    pub link: String,
    pub urlname: String,
}

// Queries all events belonging to the specified series from Redis,
// ignoring errors that might arise querying specific events
// (and not returning them instead)
pub fn get_events_for_series(
    redis_client: &mut redis::Client,
    series_id: &str,
) -> Result<Vec<Event>, super::Error> {
    let redis_series_events_key = format!("event_series:{}:meetup_events", &series_id);
    // Get all events belonging to this event series
    let event_ids: Vec<String> = redis_client.smembers(&redis_series_events_key)?;
    let events: Vec<_> = event_ids
        .into_iter()
        .filter_map(|event_id| {
            let redis_event_key = format!("meetup_event:{}", event_id);
            let res: redis::RedisResult<(String, String, String, String)> =
                redis_client.hget(&redis_event_key, &["time", "name", "link", "urlname"]);
            if let Ok((time, name, link, urlname)) = res {
                match chrono::DateTime::parse_from_rfc3339(&time) {
                    Ok(time) => Some(Event {
                        id: event_id,
                        name: name,
                        time: time.with_timezone(&chrono::Utc),
                        link: link,
                        urlname: urlname,
                    }),
                    _ => None,
                }
            } else {
                None
            }
        })
        .collect();
    Ok(events)
}
