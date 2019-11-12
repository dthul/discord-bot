use chrono::serde::ts_milliseconds;
use futures::{stream::StreamExt, Stream};
use futures_util::{stream, TryFutureExt};
use reqwest::{
    blocking::Request,
    header::{HeaderMap, AUTHORIZATION},
    Method,
};
use serde::{de::Error as _, Deserialize};

const BASE_URL: &'static str = "https://api.meetup.com";
pub const URLNAMES: [&'static str; 3] =
    ["SwissRPG-Zurich", "SwissRPG-Central", "SwissRPG-Romandie"];

#[derive(Debug, Clone)]
pub struct Client {
    client: reqwest::blocking::Client,
}

#[derive(Debug, Clone)]
pub struct AsyncClient {
    client: reqwest::Client,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Photo {
    pub thumb_link: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GroupProfile {
    pub role: Option<LeadershipRole>,
    pub status: UserStatus,
}

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    pub id: u64,
    pub name: String,
    pub photo: Option<Photo>,
    pub group_profile: Option<GroupProfile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Group {
    pub urlname: String,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum UserStatus {
    None,
    Pending,
    PendingPayment,
    Active,
    Blocked,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LeadershipRole {
    AssistantOrganizer,
    Coorganizer,
    EventOrganizer,
    Organizer,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Event {
    pub id: String, // yeah, Event IDs seem to be the only ones that are alphanumeric...
    pub name: String,
    #[serde(with = "ts_milliseconds")]
    pub time: chrono::DateTime<chrono::Utc>,
    pub event_hosts: Vec<User>,
    pub link: String,
    pub group: Group,
    pub description: String,
}

impl<'de> Deserialize<'de> for UserStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "none" => Ok(UserStatus::None),
            "pending" => Ok(UserStatus::Pending),
            "pending_payment" => Ok(UserStatus::PendingPayment),
            "active" => Ok(UserStatus::Active),
            "blocked" => Ok(UserStatus::Blocked),
            _ => Err(D::Error::invalid_value(
                serde::de::Unexpected::Enum,
                &"one of [none, pending, pending_payment, active, blocked]",
            )),
        }
    }
}

impl<'de> Deserialize<'de> for LeadershipRole {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "assistant_organizer" => Ok(LeadershipRole::AssistantOrganizer),
            "coorganizer" => Ok(LeadershipRole::Coorganizer),
            "event_organizer" => Ok(LeadershipRole::EventOrganizer),
            "organizer" => Ok(LeadershipRole::Organizer),
            _ => Err(D::Error::invalid_value(
                serde::de::Unexpected::Enum,
                &"one of [assistant_organizer, coorganizer, event_organizer, organizer]",
            )),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct UserInfo {
    pub status: UserStatus,
    pub role: Option<String>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RSVPResponse {
    Yes,
    No,
    Waitlist,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RSVP {
    pub member: User,
    pub response: RSVPResponse,
}

impl<'de> Deserialize<'de> for RSVPResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "yes" => Ok(RSVPResponse::Yes),
            "no" => Ok(RSVPResponse::No),
            "waitlist" => Ok(RSVPResponse::Waitlist),
            _ => Err(D::Error::invalid_value(
                serde::de::Unexpected::Enum,
                &"one of [yes, no, waitlist]",
            )),
        }
    }
}

impl Client {
    pub fn new(access_token: &str) -> Client {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            format!("Bearer {}", access_token).parse().unwrap(),
        );
        Client {
            client: reqwest::blocking::Client::builder()
                .default_headers(headers)
                .build()
                .expect("Could not initialize the reqwest client"),
        }
    }

    pub fn get_group_profile(&self, id: Option<u64>, urlname: &str) -> crate::Result<Option<User>> {
        let url = match id {
            Some(id) => format!(
                "{}/{}/members/{}?&sign=true&photo-host=public&only=id,name,photo,group_profile&\
                 omit=group_profile.group,group_profile.answers",
                BASE_URL, urlname, id
            ),
            _ => format!(
                "{}/{}/members/self?&sign=true&photo-host=public&only=id,name,photo,group_profile&\
                 omit=group_profile.group,group_profile.answers",
                BASE_URL, urlname
            ),
        };
        let url = url.parse()?;
        let response = self.client.execute(Request::new(Method::GET, url))?;
        if let Ok(user) = response.json::<User>() {
            return Ok(Some(user));
        } else {
            return Ok(None);
        }
    }

    pub fn get_member_profile(&self, id: Option<u64>) -> crate::Result<Option<User>> {
        let url = match id {
            Some(id) => format!(
                "{}/members/{}?&sign=true&photo-host=public&only=id,name,photo",
                BASE_URL, id
            ),
            _ => format!(
                "{}/members/self?&sign=true&photo-host=public&only=id,name,photo",
                BASE_URL
            ),
        };
        let url = url.parse()?;
        let response = self.client.execute(Request::new(Method::GET, url))?;
        if let Ok(user) = response.json::<User>() {
            return Ok(Some(user));
        } else {
            return Ok(None);
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Reqwest(reqwest::Error),
    Serde {
        error: serde_json::Error,
        input: String,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Reqwest(error) => write!(f, "Meetup Client Error (Reqwest Error):\n{:?}", error),
            Error::Serde { error, input } => write!(
                f,
                "Meetup Client Error (Deserialization Error):\n{:?}\nInput was:\n{}",
                error, input
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Reqwest(err) => Some(err),
            Error::Serde { error: err, .. } => Some(err),
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Error::Reqwest(err)
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
            client: reqwest::r#async::Client::builder()
                .default_headers(headers)
                .build()
                .expect("Could not initialize the reqwest client"),
        }
    }

    // Gets the user with the specified ID
    // TODO: currently we cannot distinguish between a non-existing user or some other kind of error
    pub async fn get_group_profile(
        &self,
        id: Option<u64>,
        urlname: &str,
    ) -> Result<Option<User>, Error> {
        let url = match id {
            Some(id) => format!(
                "{}/{}/members/{}?&sign=true&photo-host=public&only=id,name,photo,group_profile&\
                 omit=group_profile.group,group_profile.answers",
                BASE_URL, urlname, id
            ),
            _ => format!(
                "{}/{}/members/self?&sign=true&photo-host=public&only=id,name,photo,group_profile&\
                 omit=group_profile.group,group_profile.answers",
                BASE_URL, urlname
            ),
        };
        let res = self.client.get(&url).send().await?;
        let user_res = Self::try_deserialize(res).await;
        match user_res {
            Ok(user) => Ok(Some(user)),
            Err(err) => {
                // Dirty hack: instead of properly parsing the errors returned
                // by the Meetup API to figure out whether it is just a "404",
                // just look at the error text instead
                if let Error::Serde { input, .. } = &err {
                    if input.contains("member_error") {
                        return Ok(None);
                    }
                }
                Err(err)
            }
        }
    }

    // Gets the user with the specified ID
    // TODO: currently we cannot distinguish between a non-existing user or some other kind of error
    pub async fn get_member_profile(&self, id: Option<u64>) -> Result<Option<User>, Error> {
        let url = match id {
            Some(id) => format!(
                "{}/members/{}?&sign=true&photo-host=public&only=id,name,photo",
                BASE_URL, id
            ),
            _ => format!(
                "{}/members/self?&sign=true&photo-host=public&only=id,name,photo",
                BASE_URL
            ),
        };
        let res = self.client.get(&url).send().await?;
        let user_res = Self::try_deserialize(res).await;
        match user_res {
            Ok(user) => Ok(Some(user)),
            Err(err) => {
                // Dirty hack: instead of properly parsing the errors returned
                // by the Meetup API to figure out whether it is just a "404",
                // just look at the error text instead
                if let Error::Serde { input, .. } = &err {
                    if input.contains("member_error") {
                        return Ok(None);
                    }
                }
                Err(err)
            }
        }
    }

    // Doesn't implement pagination. But since Meetup returns 200 elements per page,
    // this does not matter for us anyway
    pub fn get_upcoming_events(&self, urlname: &str) -> impl Stream<Item = Result<Event, Error>> {
        let url = format!(
            "{}/{}/events?&sign=true&photo-host=public&page=200&fields=event_hosts&\
             has_ended=false&status=upcoming&only=description,event_hosts.id,event_hosts.name,id,\
             link,time,name,group.urlname",
            BASE_URL, urlname
        );
        let request = self.client.get(&url);
        request
            .send()
            .err_into::<Error>()
            .and_then(Self::try_deserialize)
            .map_ok(|event_list: Vec<Event>| stream::iter(event_list.into_iter().map(Ok)))
            .try_flatten_stream()
    }

    pub fn get_upcoming_events_all_groups(&self) -> impl Stream<Item = Result<Event, Error>> {
        let streams: Vec<_> = URLNAMES
            .iter()
            .map(|urlname| self.get_upcoming_events(urlname))
            .collect();
        stream::iter(streams).flatten()
    }

    // Get members that RSVP'd yes
    pub async fn get_rsvps(&self, urlname: &str, event_id: &str) -> Result<Vec<RSVP>, Error> {
        let url = format!(
            "{}/{}/events/{}/rsvps?&sign=true&photo-host=public&page=200&only=response,member&\
             omit=member.photo,member.event_context",
            BASE_URL, urlname, event_id
        );
        let res = self.client.get(&url).send().await?;
        Self::try_deserialize(res).await
    }

    async fn try_deserialize<T: serde::de::DeserializeOwned>(
        response: reqwest::r#async::Response,
    ) -> Result<T, Error> {
        let text = response.text().await?;
        let value: T = serde_json::from_str(&text).map_err(|err| Error::Serde {
            error: err,
            input: text,
        })?;
        Ok(value)
    }
}
