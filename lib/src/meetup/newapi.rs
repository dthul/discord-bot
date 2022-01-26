use chrono::TimeZone;
use futures::{stream, Stream, StreamExt};
use graphql_client::{GraphQLQuery, Response};
use reqwest::header::{HeaderMap, AUTHORIZATION};
use serde::de::Error as _;
use serde::Deserialize;
use serde::Serialize;
use serde::Serializer;

const API_ENDPOINT: &'static str = "https://api.meetup.com/gql";
pub const URLNAMES: [&'static str; 3] =
    ["SwissRPG-Zurich", "SwissRPG-Central", "SwissRPG-Romandie"];

#[derive(Debug, Clone)]
pub struct ZonedDateTime(pub chrono::DateTime<chrono::Utc>);

#[derive(Debug, Clone)]
pub struct AlphaNumericId(pub String);

#[derive(Debug, Clone, Copy)]
pub struct NumericId(pub u64);

pub struct Duration(chrono::Duration);

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/meetup/schema.graphql",
    query_path = "src/meetup/queries.graphql",
    response_derives = "Debug,Clone"
)]
pub struct UpcomingEventsQuery;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/meetup/schema.graphql",
    query_path = "src/meetup/queries.graphql",
    response_derives = "Debug,Clone"
)]
pub struct EventQuery;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/meetup/schema.graphql",
    query_path = "src/meetup/queries.graphql",
    response_derives = "Debug,Clone"
)]
pub struct EventTicketsQuery;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/meetup/schema.graphql",
    query_path = "src/meetup/queries.graphql",
    response_derives = "Debug,Clone"
)]
pub struct SelfQuery;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/meetup/schema.graphql",
    query_path = "src/meetup/queries.graphql",
    response_derives = "Debug,Clone,PartialEq"
)]
pub struct GroupMembershipQuery;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/meetup/schema.graphql",
    query_path = "src/meetup/queries.graphql",
    response_derives = "Debug,Clone"
)]
pub struct CreateEventMutation;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/meetup/schema.graphql",
    query_path = "src/meetup/queries.graphql",
    response_derives = "Debug,Clone"
)]
pub struct CloseEventRsvpsMutation;

pub type UpcomingEventDetails =
    upcoming_events_query::UpcomingEventsQueryGroupByUrlnameUpcomingEventsEdgesNode;

pub type Ticket = event_tickets_query::EventTicketsQueryEventTicketsEdgesNode;

pub type TicketStatus = String;

pub type NewEventResponse = create_event_mutation::CreateEventMutationCreateEventEvent;

pub type NewEvent = create_event_mutation::CreateEventInput;
pub type NewEventRsvpSettings = create_event_mutation::RsvpSettings;
pub type NewEventPublishStatus = create_event_mutation::PublishStatus;

pub type GroupMembership = group_membership_query::GroupMembershipQueryGroupByUrlname;

#[derive(Debug, Clone)]
pub struct AsyncClient {
    client: reqwest::Client,
}

#[derive(Debug, Clone)]
pub struct PayloadError {
    pub code: String,
    pub message: String,
    pub field: Option<String>,
}

#[derive(Debug)]
pub enum Error {
    Reqwest(reqwest::Error),
    GraphQL(Vec<graphql_client::Error>),
    ResourceNotFound,
    Payload(Vec<PayloadError>),
}

// TODO: using a GraphQL crate like "cynic" several queries could share the same type and we wouldn't need these conversions
impl From<NewEventResponse> for UpcomingEventDetails {
    fn from(event: NewEventResponse) -> Self {
        UpcomingEventDetails {
            id: event.id,
            title: event.title,
            event_url: event.event_url,
            short_url: event.short_url,
            description: event.description,
            hosts: event.hosts.map(|hosts| hosts.into_iter().map(|host| upcoming_events_query::UpcomingEventsQueryGroupByUrlnameUpcomingEventsEdgesNodeHosts { id: host.id }).collect()),
            date_time: event.date_time,
            max_tickets: event.max_tickets,
            going: event.going,
            is_online: event.is_online,
            rsvp_settings: event.rsvp_settings.map(|rsvp_settings| upcoming_events_query::UpcomingEventsQueryGroupByUrlnameUpcomingEventsEdgesNodeRsvpSettings { rsvps_closed: rsvp_settings.rsvps_closed }),
            venue: event.venue.map(|venue| upcoming_events_query::UpcomingEventsQueryGroupByUrlnameUpcomingEventsEdgesNodeVenue { lat: venue.lat, lng: venue.lng }),
            group: event.group.map(|group| upcoming_events_query::UpcomingEventsQueryGroupByUrlnameUpcomingEventsEdgesNodeGroup { urlname: group.urlname }),
        }
    }
}

impl From<create_event_mutation::CreateEventMutationCreateEventErrors> for PayloadError {
    fn from(error: create_event_mutation::CreateEventMutationCreateEventErrors) -> Self {
        PayloadError {
            code: error.code,
            message: error.message,
            field: error.field,
        }
    }
}

impl From<chrono::Duration> for Duration {
    fn from(duration: chrono::Duration) -> Self {
        Duration(duration)
    }
}

impl std::ops::Deref for ZonedDateTime {
    type Target = chrono::DateTime<chrono::Utc>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ZonedDateTime {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        // Sometimes Meetup returns ZonedDateTimes with an additional timzeone
        // suffix like "2022-01-16T13:43:19-05:00[US/Eastern]"
        // Before we try to decode it using chrono we remove any such timezone
        let without_suffix = match s.find('[') {
            Some(pos) => &s[..pos],
            None => &s,
        };
        let datetime = iso8601::datetime(without_suffix).ok().and_then(|datetime| {
            chrono::FixedOffset::east_opt(
                60 * (60 * datetime.time.tz_offset_hours + datetime.time.tz_offset_minutes),
            )
            .and_then(|timezone| {
                let date = match datetime.date {
                    iso8601::Date::YMD { year, month, day } => {
                        timezone.ymd_opt(year, month, day).earliest()
                    }
                    iso8601::Date::Week { year, ww, d } => {
                        let weekday = match d {
                            1 => Some(chrono::Weekday::Mon),
                            2 => Some(chrono::Weekday::Tue),
                            3 => Some(chrono::Weekday::Wed),
                            4 => Some(chrono::Weekday::Thu),
                            5 => Some(chrono::Weekday::Fri),
                            6 => Some(chrono::Weekday::Sat),
                            7 => Some(chrono::Weekday::Sun),
                            _ => None,
                        };
                        weekday
                            .and_then(|weekday| timezone.isoywd_opt(year, ww, weekday).earliest())
                    }
                    iso8601::Date::Ordinal { year, ddd } => timezone.yo_opt(year, ddd).earliest(),
                };
                date.and_then(|date| {
                    date.and_hms_opt(
                        datetime.time.hour,
                        datetime.time.minute,
                        datetime.time.second,
                    )
                })
            })
        });
        if let Some(datetime) = datetime {
            Ok(ZonedDateTime(datetime.with_timezone(&chrono::Utc)))
        } else {
            Err(D::Error::invalid_value(
                serde::de::Unexpected::Str(&s),
                &"a date time string like '2022-01-16T13:43:19-05:00'",
            ))
        }
    }
}

impl Serialize for ZonedDateTime {
    /// Serialize into a rfc3339 time string
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl Serialize for Duration {
    /// Serialize into an ISO 8601 duration
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(format!("PT{}S", self.0.num_seconds()).as_str())
    }
}

impl<'de> Deserialize<'de> for AlphaNumericId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        // Sometimes Meetup returns IDs with a "!chp" suffix
        // We strip it here
        if s.ends_with("!chp") {
            Ok(AlphaNumericId(s[..s.len() - 4].into()))
        } else {
            Ok(AlphaNumericId(s))
        }
    }
}

impl<'de> Deserialize<'de> for NumericId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        // Sometimes Meetup returns IDs with a "!chp" suffix
        // We strip it here
        let without_suffix = if s.ends_with("!chp") {
            &s[..s.len() - 4]
        } else {
            &s
        };
        match without_suffix.parse::<u64>() {
            Ok(id) => Ok(NumericId(id)),
            Err(_err) => {
                return Err(D::Error::invalid_value(
                    serde::de::Unexpected::Str(&s),
                    &"a numeric ID like '7594361' or '7594361!chp'",
                ))
            }
        }
    }
}

impl UpcomingEventDetails {
    pub fn num_free_spots(&self) -> u32 {
        (self.max_tickets - self.going).max(0) as u32
    }

    pub fn host_ids(&self) -> Vec<u64> {
        self.hosts
            .as_ref()
            .map(|hosts| hosts.iter().map(|user| user.id.0).collect())
            .unwrap_or(vec![])
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Reqwest(error) => write!(
                f,
                "Meetup GraphQL Client Error (Reqwest Error):\n{:?}",
                error
            ),
            Error::GraphQL(error) => write!(f, "Meetup GraphQL Client Error:\n{:#?}", error),
            Error::ResourceNotFound => write!(f, "Resource not found"),
            Error::Payload(errors) => write!(f, "Payload errors:\n{:#?}", errors),
        }
    }
}

impl std::fmt::Display for AlphaNumericId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Display for NumericId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Reqwest(err) => Some(err),
            Error::GraphQL(..) => None,
            Error::ResourceNotFound => None,
            Error::Payload(..) => None,
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Error::Reqwest(err)
    }
}

impl self_query::SelfQuerySelfMemberPhoto {
    pub fn url_for_size(&self, width: u16, height: u16) -> Option<String> {
        if let Some(base_url) = &self.base_url {
            Some(format!("{base_url}{id}/{width}x{height}.jpg", id = self.id))
        } else {
            None
        }
    }
}

impl AsyncClient {
    pub fn new(access_token: &str) -> AsyncClient {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            format!("Bearer {}", access_token).parse().unwrap(),
        );
        AsyncClient {
            client: reqwest::Client::builder()
                .default_headers(headers)
                .build()
                .expect("Could not initialize the reqwest client"),
        }
    }

    pub async fn get_event(&self, id: String) -> Result<event_query::EventQueryEvent, Error> {
        use event_query::*;
        let query_variables = Variables { id };
        let query = EventQuery::build_query(query_variables);
        let http_response = self.client.post(API_ENDPOINT).json(&query).send().await?;
        let response: Response<ResponseData> = http_response.json().await?;
        match response.data {
            Some(ResponseData { event: Some(event) }) => Ok(event),
            Some(ResponseData { event: None }) => Err(Error::ResourceNotFound),
            _ => Err(Error::GraphQL(response.errors.unwrap_or(vec![]))),
        }
    }

    pub fn get_upcoming_events<'a>(
        &'a self,
        urlname: String,
    ) -> impl Stream<Item = Result<UpcomingEventDetails, Error>> + 'a {
        use upcoming_events_query::*;
        enum States {
            QueryPage {
                cursor: Option<String>,
            },
            YieldEvents {
                next_page_cursor: Option<String>,
                events: Vec<UpcomingEventDetails>,
            },
            End,
        }
        const NUM_EVENTS_PER_QUERY: i64 = 20;
        let state = States::QueryPage { cursor: None };
        futures::stream::unfold(state, move |mut state| {
            let urlname = urlname.clone();
            async move {
                loop {
                    state = match state {
                        States::End => return None,
                        States::YieldEvents {
                            next_page_cursor,
                            mut events,
                        } => {
                            if let Some(event) = events.pop() {
                                return Some((
                                    Ok(event),
                                    States::YieldEvents {
                                        next_page_cursor,
                                        events,
                                    },
                                ));
                            } else {
                                if let Some(cursor) = next_page_cursor {
                                    States::QueryPage {
                                        cursor: Some(cursor),
                                    }
                                } else {
                                    States::End
                                }
                            }
                        }
                        States::QueryPage { cursor } => {
                            // Query the next page of upcoming events
                            let query_variables = Variables {
                                urlname: urlname.clone(),
                                first: NUM_EVENTS_PER_QUERY,
                                after: cursor,
                            };
                            let query = UpcomingEventsQuery::build_query(query_variables);
                            let http_response =
                                match self.client.post(API_ENDPOINT).json(&query).send().await {
                                    Err(error) => return Some((Err(error.into()), States::End)),
                                    Ok(response) => response,
                                };
                            let response: Response<ResponseData> = match http_response.json().await
                            {
                                Err(error) => return Some((Err(error.into()), States::End)),
                                Ok(response) => response,
                            };
                            match response.data {
                                Some(ResponseData {
                                    group_by_urlname: None,
                                }) => return Some((Err(Error::ResourceNotFound), States::End)),
                                Some(ResponseData {
                                    group_by_urlname:
                                        Some(UpcomingEventsQueryGroupByUrlname { upcoming_events }),
                                }) => {
                                    let next_page_cursor =
                                        if upcoming_events.page_info.has_next_page {
                                            // There are more result pages, continue querying
                                            Some(upcoming_events.page_info.end_cursor)
                                        } else {
                                            // There are no more results, we are done
                                            None
                                        };
                                    let events = upcoming_events
                                        .edges
                                        .into_iter()
                                        .map(|edge| edge.node)
                                        .collect();
                                    States::YieldEvents {
                                        next_page_cursor,
                                        events,
                                    }
                                }
                                _ => {
                                    return Some((
                                        Err(Error::GraphQL(response.errors.unwrap_or(vec![]))),
                                        States::End,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        })
    }

    pub fn get_upcoming_events_all_groups<'a>(
        &'a self,
    ) -> impl Stream<Item = Result<UpcomingEventDetails, Error>> + 'a {
        stream::iter(URLNAMES).flat_map(|urlname| self.get_upcoming_events(urlname.into()))
    }

    pub fn get_tickets<'a>(
        &'a self,
        event_id: String,
    ) -> impl Stream<Item = Result<Ticket, Error>> + 'a {
        use event_tickets_query::*;
        enum States {
            QueryPage {
                cursor: Option<String>,
            },
            YieldTickets {
                next_page_cursor: Option<String>,
                tickets: Vec<Ticket>,
            },
            End,
        }
        const NUM_TICKETS_PER_QUERY: i64 = 10;
        let state = States::QueryPage { cursor: None };
        futures::stream::unfold(state, move |mut state| {
            let event_id = event_id.clone();
            async move {
                loop {
                    state = match state {
                        States::End => return None,
                        States::YieldTickets {
                            next_page_cursor,
                            mut tickets,
                        } => {
                            if let Some(ticket) = tickets.pop() {
                                return Some((
                                    Ok(ticket),
                                    States::YieldTickets {
                                        next_page_cursor,
                                        tickets,
                                    },
                                ));
                            } else {
                                if let Some(cursor) = next_page_cursor {
                                    States::QueryPage {
                                        cursor: Some(cursor),
                                    }
                                } else {
                                    States::End
                                }
                            }
                        }
                        States::QueryPage { cursor } => {
                            // Query the next page of tickets
                            let query_variables = Variables {
                                id: event_id.clone(),
                                first: NUM_TICKETS_PER_QUERY,
                                after: cursor,
                            };
                            let query = EventTicketsQuery::build_query(query_variables);
                            let http_response =
                                match self.client.post(API_ENDPOINT).json(&query).send().await {
                                    Err(error) => return Some((Err(error.into()), States::End)),
                                    Ok(response) => response,
                                };
                            let response: Response<ResponseData> = match http_response.json().await
                            {
                                Err(error) => return Some((Err(error.into()), States::End)),
                                Ok(response) => response,
                            };
                            match response.data {
                                Some(ResponseData { event: None }) => {
                                    return Some((Err(Error::ResourceNotFound), States::End))
                                }
                                Some(ResponseData {
                                    event: Some(EventTicketsQueryEvent { tickets, .. }),
                                }) => {
                                    let next_page_cursor = if tickets.page_info.has_next_page {
                                        // There are more result pages, continue querying
                                        Some(tickets.page_info.end_cursor)
                                    } else {
                                        // There are no more results, we are done
                                        None
                                    };
                                    let tickets =
                                        tickets.edges.into_iter().map(|edge| edge.node).collect();
                                    States::YieldTickets {
                                        next_page_cursor,
                                        tickets,
                                    }
                                }
                                _ => {
                                    return Some((
                                        Err(Error::GraphQL(response.errors.unwrap_or(vec![]))),
                                        States::End,
                                    ))
                                }
                            }
                        }
                    }
                }
            }
        })
    }

    pub async fn get_tickets_vec(&self, event_id: String) -> Result<Vec<Ticket>, Error> {
        let ticket_stream = self.get_tickets(event_id);
        let mut tickets = vec![];
        futures::pin_mut!(ticket_stream);
        while let Some(value) = ticket_stream.next().await {
            match value {
                Ok(ticket) => tickets.push(ticket),
                Err(error) => return Err(error.into()),
            }
        }
        Ok(tickets)
    }

    pub async fn get_self(&self) -> Result<self_query::SelfQuerySelf, Error> {
        use self_query::*;
        let query = SelfQuery::build_query(Variables {});
        let http_response = self.client.post(API_ENDPOINT).json(&query).send().await?;
        let response: Response<ResponseData> = http_response.json().await?;
        match response.data {
            Some(ResponseData { self_: Some(self_) }) => Ok(self_),
            Some(ResponseData { self_: None }) => Err(Error::ResourceNotFound),
            _ => Err(Error::GraphQL(response.errors.unwrap_or(vec![]))),
        }
    }

    pub async fn create_event(&self, new_event: NewEvent) -> Result<NewEventResponse, Error> {
        use create_event_mutation::*;
        let query_variables = Variables { input: new_event };
        let query = CreateEventMutation::build_query(query_variables);
        let http_response = self.client.post(API_ENDPOINT).json(&query).send().await?;
        let response: Response<ResponseData> = http_response.json().await?;
        match response.data {
            Some(ResponseData {
                create_event:
                    CreateEventMutationCreateEvent {
                        event: Some(event), ..
                    },
            }) => Ok(event),
            Some(ResponseData {
                create_event:
                    CreateEventMutationCreateEvent {
                        event: None,
                        errors: Some(errors),
                    },
            }) => Err(Error::Payload(errors.into_iter().map(Into::into).collect())),
            _ => Err(Error::GraphQL(response.errors.unwrap_or(vec![]))),
        }
    }

    pub async fn get_group_membership(&self, urlname: String) -> Result<GroupMembership, Error> {
        use group_membership_query::*;
        let query_variables = Variables { urlname: urlname };
        let query = GroupMembershipQuery::build_query(query_variables);
        let http_response = self.client.post(API_ENDPOINT).json(&query).send().await?;
        let response: Response<ResponseData> = http_response.json().await?;
        match response.data {
            Some(ResponseData {
                group_by_urlname: Some(group_membership),
            }) => Ok(group_membership),
            Some(ResponseData {
                group_by_urlname: None,
            }) => Err(Error::ResourceNotFound),
            _ => Err(Error::GraphQL(response.errors.unwrap_or(vec![]))),
        }
    }

    pub async fn close_rsvps(&self, event_id: String) -> Result<(), Error> {
        use close_event_rsvps_mutation::*;
        let query_variables = Variables {
            input: CloseEventRsvpsInput { eventId: event_id },
        };
        let query = CloseEventRsvpsMutation::build_query(query_variables);
        let http_response = self.client.post(API_ENDPOINT).json(&query).send().await?;
        let response: Response<ResponseData> = http_response.json().await?;
        match response.data {
            Some(ResponseData {
                close_event_rsvps:
                    CloseEventRsvpsMutationCloseEventRsvps {
                        event: Some(..), ..
                    },
            }) => Ok(()),
            Some(ResponseData {
                close_event_rsvps: CloseEventRsvpsMutationCloseEventRsvps { event: None, .. },
            }) => Err(Error::ResourceNotFound),
            _ => Err(Error::GraphQL(response.errors.unwrap_or(vec![]))),
        }
    }
}
