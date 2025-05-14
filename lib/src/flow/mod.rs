use futures_util::FutureExt;
use rand::Rng;
use redis::AsyncCommands;

use crate::{db, meetup::newapi::create_event_mutation::CreateEventInput};

pub struct ScheduleSessionFlow {
    pub id: u64,
    pub event_series_id: db::EventSeriesId,
}

impl ScheduleSessionFlow {
    pub async fn new(
        redis_connection: &mut redis::aio::MultiplexedConnection,
        event_series_id: db::EventSeriesId,
    ) -> Result<Self, crate::meetup::Error> {
        let id: u64 = rand::thread_rng().gen();
        let redis_key = format!("flow:schedule_session:{}", id);
        let mut pipe = redis::pipe();
        let _: () = pipe
            .hset(&redis_key, "event_series_id", event_series_id.0)
            .ignore()
            .expire(&redis_key, 10 * 60)
            .query_async(redis_connection)
            .await?;
        Ok(ScheduleSessionFlow {
            id,
            event_series_id,
        })
    }

    pub async fn retrieve(
        redis_connection: &mut redis::aio::MultiplexedConnection,
        id: u64,
    ) -> Result<Option<Self>, crate::meetup::Error> {
        let redis_key = format!("flow:schedule_session:{}", id);
        let event_series_id: Option<i32> =
            redis_connection.hget(&redis_key, "event_series_id").await?;
        let flow = event_series_id.map(|event_series_id| ScheduleSessionFlow {
            id: id,
            event_series_id: db::EventSeriesId(event_series_id),
        });
        Ok(flow)
    }

    pub async fn schedule<'a>(
        self,
        db_connection: sqlx::PgPool,
        mut redis_connection: redis::aio::MultiplexedConnection,
        meetup_client: &'a crate::meetup::newapi::AsyncClient,
        // oauth2_consumer: &'a crate::meetup::oauth2::OAuth2Consumer,
        date_time: chrono::DateTime<chrono::Utc>,
        is_open_event: bool,
    ) -> Result<crate::meetup::newapi::NewEventResponse, crate::meetup::Error> {
        let events = db::get_events_for_series(&db_connection, self.event_series_id).await?;
        let latest_event = if let Some(event) = events.first() {
            event
        } else {
            return Err(simple_error::SimpleError::new(
                "Could not find an existing event to schedule a follow up session for",
            )
            .into());
        };
        // Find the latest event in the series which was published on Meetup
        let latest_meetup_event = events.iter().find_map(|event| {
            if let Some(meetup_event) = &event.meetup_event {
                Some(meetup_event)
            } else {
                None
            }
        });
        let latest_meetup_event = if let Some(event) = latest_meetup_event {
            event
        } else {
            return Err(
                simple_error::SimpleError::new("Could not find a Meetup event to clone").into(),
            );
        };
        // Clone the Meetup event
        let new_event_hook = Box::new(|mut new_event: CreateEventInput| {
            new_event.title = latest_event.title.clone();
            new_event.description = latest_event.description.clone();
            // TODO: hosts from latest session?
            Self::new_event_hook(
                new_event,
                date_time,
                &latest_meetup_event.meetup_id,
                is_open_event,
            )
        }) as _;
        let new_event = crate::meetup::util::clone_event(
            &latest_meetup_event.urlname,
            &latest_meetup_event.meetup_id,
            meetup_client,
            Some(new_event_hook),
        )
        .await?;
        // // Try to transfer the RSVPs to the new event
        // let rsvp_result = match crate::meetup::util::clone_rsvps(
        //     &latest_event.urlname,
        //     &latest_event.id,
        //     &new_event.id,
        //     &mut redis_connection,
        //     meetup_client,
        //     oauth2_consumer,
        // )
        // .await
        // {
        //     Ok(result) => Some(result),
        //     Err(err) => {
        //         eprintln!("Could not transfer all RSVPs to the new event.\n{:#?}", err);
        //         None
        //     }
        // };
        let redis_key = format!("flow:schedule_session:{}", self.id);
        let _: redis::RedisResult<()> = redis_connection.del(&redis_key).await;
        let sync_future = {
            let new_event = new_event.clone();
            // let rsvps = rsvp_result.as_ref().map(|res| res.cloned_rsvps.clone());
            async move {
                crate::meetup::sync::sync_event(new_event.into(), &db_connection).await?;
                // if let Some(rsvps) = rsvps {
                //     crate::meetup::sync::sync_rsvps(&event_id, rsvps, &mut redis_connection)
                //         .await?;
                // }
                Ok::<_, crate::meetup::Error>(())
            }
        };
        tokio::spawn(sync_future.map(|res| {
            if let Err(err) = res {
                eprintln!("Could not sync the newly scheduled event:\n{:#?}", err);
            }
        }));
        Ok(new_event)
    }

    pub async fn delete(
        self,
        redis_connection: &mut redis::aio::MultiplexedConnection,
    ) -> Result<(), crate::meetup::Error> {
        let redis_key = format!("flow:schedule_session:{}", self.id);
        let () = redis_connection.del(&redis_key).await?;
        Ok(())
    }

    pub fn new_event_hook(
        mut new_event: crate::meetup::newapi::NewEvent,
        new_date_time: chrono::DateTime<chrono::Utc>,
        old_event_id: &str,
        is_open_event: bool,
    ) -> Result<crate::meetup::newapi::NewEvent, crate::meetup::Error> {
        // Remove unnecessary shortcodes from follow-up sessions
        let description = new_event.description;
        let description = crate::meetup::sync::NEW_ADVENTURE_REGEX.replace_all(&description, "");
        let description = crate::meetup::sync::NEW_CAMPAIGN_REGEX.replace_all(&description, "");
        // We don't remove the [online] shortcode from descriptions anymore,
        // such that the "free game spots" feature has an easy way to tell
        // whether an event is online or not. This is mostly due to the fact
        // that at the time of this writing, we can not use the official Meetup
        // feature (yet?) for marking events as being online.
        // let description = crate::meetup::sync::ONLINE_REGEX.replace_all(&description, "");
        let mut description = crate::meetup::sync::CHANNEL_REGEX
            .replace_all(&description, "")
            .into_owned();
        // If this event is an "open event", make sure that there is no [closed] shortcode.
        // (We don't add it automatically here for closed events though)
        if is_open_event {
            description = crate::free_spots::CLOSED_REGEX
                .replace_all(&description, "")
                .into_owned()
        }
        // Add an event series shortcode if there is none yet
        if !crate::meetup::sync::EVENT_SERIES_REGEX.is_match(&description) {
            description.push_str(&format!("\n[campaign {}]", old_event_id));
        }
        // Increase the Session number
        let title_captures = crate::meetup::sync::SESSION_REGEX.captures_iter(&new_event.title);
        // Match the rightmost occurence of " Session X" in the event name.
        // Returns the event name without the session number (title_only) and
        // the current session number
        let (title_only, session_number) = if let Some(capture) = title_captures.last() {
            // If there is a match, increase the number
            // Extract the current number from the title
            let session_number = capture.name("number").unwrap().as_str();
            // Try to parse the session number
            let session_number = session_number.parse::<i32>()?;
            // Find the range of the " Session X" match and remove it from the string
            let session_x_match = capture.get(0).unwrap();
            let mut title_only = new_event.title.clone();
            title_only.truncate(session_x_match.start());
            (title_only, session_number)
        } else {
            // If there is no match, return the whole name and Session number 1
            (new_event.title.clone(), 1)
        };
        // Create a new " Session X+1" suffix
        let new_session_suffix = format!(" Session {}", session_number + 1);
        // Check if the concatenation of event title and session suffix is short enough
        let new_event_title = if title_only.encode_utf16().count()
            + new_session_suffix.encode_utf16().count()
            <= crate::meetup::MAX_EVENT_NAME_UTF16_LEN
        {
            title_only + &new_session_suffix
        } else {
            // Event title and session prefix together are too long.
            // Shorten the event title and add an ellipsis.
            let ellipsis = "…";
            let ellipsis_utf16_len = ellipsis.encode_utf16().count();
            let max_title_utf16_len = crate::meetup::MAX_EVENT_NAME_UTF16_LEN
                - new_session_suffix.encode_utf16().count()
                - ellipsis_utf16_len;
            let shortened_title =
                crate::meetup::util::truncate_str(title_only, max_title_utf16_len);
            shortened_title + ellipsis + &new_session_suffix
        };
        new_event.title = new_event_title;
        new_event.description = description;
        new_event.start_date_time = crate::meetup::newapi::DateTime(new_date_time);
        Ok(new_event)
    }
}
