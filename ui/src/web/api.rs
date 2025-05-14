use std::{ops::Deref, sync::Arc};

use axum::{
    async_trait,
    extract::{Extension, FromRequestParts, Path},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use axum_extra::{headers::Header, TypedHeader};
use hyper::http::request::Parts;
use lazy_static::lazy_static;
use lib::db::get_meetup_events_participants;
use serde::Serialize;
use serenity::model::prelude::UserId;

use super::{server::State, WebError};

pub fn create_routes() -> Router {
    Router::new()
        .route(
            "/check_discord_username",
            get(check_discord_username_handler),
        )
        .route("/list_players/:meetup_event_id", get(list_players_handler))
}

struct ApiKeyHeader(String);

lazy_static! {
    static ref API_KEY_HEADER: axum_extra::headers::HeaderName =
        axum_extra::headers::HeaderName::from_lowercase(b"api-key").unwrap();
}

impl Deref for ApiKeyHeader {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Header for ApiKeyHeader {
    fn name() -> &'static axum_extra::headers::HeaderName {
        &API_KEY_HEADER
    }

    fn decode<'i, I>(values: &mut I) -> Result<Self, axum_extra::headers::Error>
    where
        Self: Sized,
        I: Iterator<Item = &'i axum_extra::headers::HeaderValue>,
    {
        let value = values
            .next()
            .ok_or_else(axum_extra::headers::Error::invalid)?;
        let value = value
            .to_str()
            .map_err(|_| axum_extra::headers::Error::invalid())?;
        Ok(ApiKeyHeader(value.into()))
    }

    fn encode<E: Extend<axum_extra::headers::HeaderValue>>(&self, values: &mut E) {
        match axum_extra::headers::HeaderValue::from_str(&self.0) {
            Ok(header_value) => values.extend(Some(header_value)),
            Err(err) => eprintln!("Failed to encode Api-Key HTTP header: {:#?}", err),
        }
    }
}

struct DiscordUsernameHeader(String);

lazy_static! {
    static ref DISCORD_USERNAME_HEADER: axum_extra::headers::HeaderName =
        axum_extra::headers::HeaderName::from_lowercase(b"discord-username").unwrap();
}

impl Deref for DiscordUsernameHeader {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Header for DiscordUsernameHeader {
    fn name() -> &'static axum_extra::headers::HeaderName {
        &DISCORD_USERNAME_HEADER
    }

    fn decode<'i, I>(values: &mut I) -> Result<Self, axum_extra::headers::Error>
    where
        Self: Sized,
        I: Iterator<Item = &'i axum_extra::headers::HeaderValue>,
    {
        let value = values
            .next()
            .ok_or_else(axum_extra::headers::Error::invalid)?;
        let value = value
            .to_str()
            .map_err(|_| axum_extra::headers::Error::invalid())?;
        Ok(DiscordUsernameHeader(value.into()))
    }

    fn encode<E: Extend<axum_extra::headers::HeaderValue>>(&self, values: &mut E) {
        match axum_extra::headers::HeaderValue::from_str(&self.0) {
            Ok(header_value) => values.extend(Some(header_value)),
            Err(err) => eprintln!("Failed to encode Discord-Username HTTP header: {:#?}", err),
        }
    }
}

struct ApiKeyIsValid;

#[async_trait]
impl<S> FromRequestParts<S> for ApiKeyIsValid
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let TypedHeader(api_key) = TypedHeader::<ApiKeyHeader>::from_request_parts(parts, state)
            .await
            .map_err(|err| err.into_response())?;
        let Extension(state): Extension<Arc<State>> = Extension::from_request_parts(parts, state)
            .await
            .map_err(|err| err.into_response())?;
        if state.api_keys.iter().any(|key| *key == *api_key) {
            Ok(ApiKeyIsValid)
        } else {
            Err(StatusCode::UNAUTHORIZED.into_response())
        }
    }
}

async fn check_discord_username_handler(
    _: ApiKeyIsValid,
    TypedHeader(discord_username): TypedHeader<DiscordUsernameHeader>,
    Extension(state): Extension<Arc<State>>,
) -> Result<StatusCode, WebError> {
    let id = lib::tasks::subscription_roles::discord_username_to_id(
        &state.discord_cache_http,
        &discord_username.0,
    )
    .await?;
    if id.is_none() {
        // The username seems to be invalid, return a 204 HTTP code
        Ok(StatusCode::NO_CONTENT)
    } else {
        // The username could be matched to an ID, return a 200 HTTP code
        Ok(StatusCode::OK)
    }
}

#[derive(Serialize)]
struct ListPlayersEntry {
    meetup_id: Option<u64>,
    discord_id: Option<UserId>,
    discord_nick: Option<String>,
    is_host: bool,
}

async fn list_players_handler(
    _: ApiKeyIsValid,
    Path(meetup_event_id): Path<String>,
    Extension(state): Extension<Arc<State>>,
) -> Result<Json<Vec<ListPlayersEntry>>, WebError> {
    let meetup_event_ids = &[meetup_event_id];
    let players =
        get_meetup_events_participants(meetup_event_ids, /*hosts*/ false, &state.pool).await?;
    let hosts =
        get_meetup_events_participants(meetup_event_ids, /*hosts*/ true, &state.pool).await?;
    let players = players.into_iter().map(|player| ListPlayersEntry {
        meetup_id: player.meetup_id,
        discord_id: player.discord_id,
        discord_nick: player.discord_nick,
        is_host: false,
    });
    let hosts = hosts.into_iter().map(|host| ListPlayersEntry {
        meetup_id: host.meetup_id,
        discord_id: host.discord_id,
        discord_nick: host.discord_nick,
        is_host: true,
    });
    Ok(Json(hosts.chain(players).collect()))
}
