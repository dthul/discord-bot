use std::{borrow::Cow, sync::Arc};

use askama::Template;
use askama_axum::IntoResponse;
use axum::{
    body,
    extract::{Extension, OriginalUri, Path, Query},
    headers::HeaderMap,
    response::Response,
    routing::get,
    Router,
};
use cookie::Cookie;
use lib::{DefaultStr, LinkingAction, LinkingMemberDiscord, LinkingMemberMeetup, LinkingResult};
use oauth2::{AuthorizationCode, CsrfToken, RedirectUrl, Scope, TokenResponse};
use redis::AsyncCommands;
use serde::Deserialize;
use serenity::model::id::UserId;

use super::{server::State, MessageTemplate, WebError};

pub fn create_routes() -> Router {
    Router::new()
        .route("/authorize", get(authorize_handler))
        .route("/authorize/redirect", get(authorize_redirect_handler))
        .route("/link/:linking_id", get(link_handler))
        .route(
            "/link/:linking_id/rsvp/redirect",
            get(link_redirect_handler).layer(Extension(WithRsvpScope(true))),
        )
        .route(
            "/link/:linking_id/norsvp/redirect",
            get(link_redirect_handler).layer(Extension(WithRsvpScope(false))),
        )
}

#[derive(Deserialize)]
struct LinkQuery {
    code: String,
    state: String,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WithRsvpScope(pub bool);

#[derive(Template)]
#[template(path = "link.html")]
struct LinkingTemplate<'a> {
    authorize_url: &'a str,
}

async fn generate_csrf_cookie(
    redis_connection: &mut redis::aio::MultiplexedConnection,
    csrf_state: &str,
) -> Result<Cookie<'static>, lib::meetup::Error> {
    let random_csrf_user_id = lib::new_random_id(16);
    let redis_csrf_key = format!("csrf:{}", &random_csrf_user_id);
    let _: () = redis_connection
        .set_ex(&redis_csrf_key, csrf_state, 3600)
        .await?;
    Ok(Cookie::build(("csrf_user_id", random_csrf_user_id))
        .domain(lib::urls::DOMAIN)
        .http_only(true)
        .same_site(cookie::SameSite::Lax)
        .max_age(cookie::time::Duration::hours(1))
        .into())
}

async fn check_csrf_cookie(
    redis_connection: &mut redis::aio::MultiplexedConnection,
    headers: &hyper::HeaderMap<hyper::header::HeaderValue>,
    csrf_state: &str,
) -> Result<bool, lib::meetup::Error> {
    let csrf_user_id_cookie =
        headers
            .get_all(hyper::header::COOKIE)
            .iter()
            .find_map(|header_value| {
                if let Ok(header_value) = header_value.to_str() {
                    if let Ok(cookie) = Cookie::parse(header_value) {
                        if cookie.name() == "csrf_user_id" {
                            return Some(cookie);
                        }
                    }
                }
                None
            });
    let csrf_user_id_cookie = match csrf_user_id_cookie {
        None => return Ok(false),
        Some(csrf_user_id_cookie) => csrf_user_id_cookie,
    };
    let redis_csrf_key = format!("csrf:{}", csrf_user_id_cookie.value_trimmed());
    let csrf_stored_state: Option<String> = redis_connection.get(&redis_csrf_key).await?;
    let csrf_stored_state: String = match csrf_stored_state {
        None => return Ok(false),
        Some(csrf_stored_state) => csrf_stored_state,
    };
    Ok(csrf_state == csrf_stored_state)
}

async fn authorize_handler(Extension(state): Extension<Arc<State>>) -> Result<Response, WebError> {
    let mut redis_connection = state
        .redis_client
        .get_multiplexed_async_connection()
        .await?;
    // Generate the authorization URL to which we'll redirect the user.
    let (authorize_url, csrf_state) = state
        .oauth2_consumer
        .authorization_client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("ageless".to_string()))
        .add_scope(Scope::new("basic".to_string()))
        .add_scope(Scope::new("event_management".to_string()))
        .url();
    // Store the generated CSRF token so we can compare it to the one
    // returned by Meetup later
    let csrf_cookie = generate_csrf_cookie(&mut redis_connection, csrf_state.secret()).await?;
    let mut response = MessageTemplate {
        title: Cow::Borrowed("Login with Meetup"),
        safe_content: Some(Cow::Owned(format!(
            "<a href=\"{}\">Login with Meetup</a>",
            authorize_url
        ))),
        content: None,
        img_url: None,
    }
    .into_response();
    response.headers_mut().insert(
        hyper::header::SET_COOKIE,
        csrf_cookie.to_string().try_into()?,
    );
    Ok(response)
}

async fn authorize_redirect_handler(
    Extension(state): Extension<Arc<State>>,
    Query(query): Query<LinkQuery>,
    headers: HeaderMap,
) -> Result<MessageTemplate, WebError> {
    if let Some(error) = query.error {
        return Ok(("OAuth2 error", error).into());
    }
    // let mut redis_connection = state.redis_client.get_multiplexed_async_connection().await?;
    // Compare the CSRF state that was returned by Meetup to the one
    // we have saved
    // let csrf_is_valid = check_csrf_cookie(&mut redis_connection, &headers, &query.state).await?;
    // if !csrf_is_valid {
    //     return Ok((
    //         "CSRF check failed",
    //         "Please go back to the first page, reload, and repeat the process",
    //     )
    //         .into());
    // }
    // Exchange the code with a token.
    let code = AuthorizationCode::new(query.code);
    let async_meetup_client = state.async_meetup_client.clone();
    let token_res = state
        .oauth2_consumer
        .authorization_client
        .exchange_code(code)
        .request_async(oauth2::reqwest::async_http_client)
        .await?;
    // Check that this token belongs to an organizer of all our Meetup groups
    let new_async_meetup_client =
        lib::meetup::newapi::AsyncClient::new(token_res.access_token().secret());
    let user_memberships =
        lib::meetup::util::get_group_memberships(new_async_meetup_client.clone()).await?;
    let is_organizer = user_memberships.iter().all(|membership| {
        use lib::meetup::newapi::group_membership_query::*;
        let is_organizer = match &membership.membership_metadata {
            Some(GroupMembershipQueryGroupByUrlnameMembershipMetadata {
                status,
                role: Some(role),
            }) => {
                (status == &MembershipStatus::ACTIVE || status == &MembershipStatus::LEADER)
                    && (role == &MemberRole::ORGANIZER
                        || role == &MemberRole::COORGANIZER
                        || role == &MemberRole::ASSISTANT_ORGANIZER)
            }
            _ => false,
        };
        is_organizer
    });
    if !is_organizer {
        return Ok(("Only the organizer can log in", "").into());
    }
    // Store the new access and refresh tokens
    let mut tx = state.pool.begin().await?;
    sqlx::query!(r#"DELETE FROM organizer_token"#)
        .execute(&mut *tx)
        .await?;
    sqlx::query!(
        r#"INSERT INTO organizer_token (meetup_access_token, meetup_refresh_token, meetup_access_token_refresh_time) VALUES ($1, $2, $3)"#,
        token_res.access_token().secret(),
        token_res.refresh_token().map(|token| token.secret()),
        chrono::Utc::now() + chrono::Duration::days(2))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    // Replace the meetup client
    *async_meetup_client.lock().await = Some(Arc::new(new_async_meetup_client));
    Ok(("Thanks for logging in :)", "").into())
}

async fn link_handler(
    Extension(state): Extension<Arc<State>>,
    Path(linking_id): Path<String>,
) -> Result<Response, WebError> {
    // The linking ID was stored in Redis when the linking link was created.
    // Check that it is still valid
    let mut redis_connection = state
        .redis_client
        .get_multiplexed_async_connection()
        .await?;
    let redis_key = format!("meetup_linking:{}:discord_user", linking_id);
    let mut pipe = redis::pipe();
    pipe.expire(&redis_key, 600).ignore().get(&redis_key);
    let (discord_id,): (Option<u64>,) = pipe.query_async(&mut redis_connection).await?;
    if discord_id.is_none() {
        let template: MessageTemplate = (
            lib::strings::OAUTH2_LINK_EXPIRED_TITLE,
            lib::strings::OAUTH2_LINK_EXPIRED_CONTENT,
        )
            .into();
        return Ok(template.into_response());
    }
    // TODO: check that this Discord ID is not linked yet before generating an authorization URL
    // Generate the authorization URL to which we'll redirect the user.
    // Two versions: One with just the "basic" scope to identify the user.
    // The second with the "rsvp" scope that will allow us to RSVP the user to events.
    let csrf_state = CsrfToken::new_random();
    let (_authorize_url_basic, csrf_state) = (*state.oauth2_consumer.link_client)
        .clone()
        .set_redirect_uri(RedirectUrl::new(format!(
            "{}/link/{}/norsvp/redirect",
            lib::urls::BASE_URL,
            linking_id
        ))?)
        .authorize_url(|| csrf_state)
        .add_scope(Scope::new("basic".to_string()))
        .url();
    let (authorize_url_rsvp, csrf_state) = (*state.oauth2_consumer.link_client)
        .clone()
        .set_redirect_uri(RedirectUrl::new(format!(
            "{}/link/{}/rsvp/redirect",
            lib::urls::BASE_URL,
            linking_id
        ))?)
        .authorize_url(|| csrf_state)
        .add_scope(Scope::new("basic".to_string()))
        .add_scope(Scope::new("rsvp".to_string()))
        .url();
    // Store the generated CSRF token so we can compare it to the one
    // returned by Meetup later
    let csrf_cookie = generate_csrf_cookie(&mut redis_connection, csrf_state.secret()).await?;
    let linking_template = LinkingTemplate {
        authorize_url: authorize_url_rsvp.as_str(),
    };
    let html_body = linking_template.render()?;
    let response = Response::builder()
        .header(hyper::header::SET_COOKIE, csrf_cookie.to_string())
        .body(body::boxed(body::Full::from(html_body)))?;
    Ok(response)
}

async fn link_redirect_handler(
    Extension(state): Extension<Arc<State>>,
    OriginalUri(path): OriginalUri,
    Query(query): Query<LinkQuery>,
    // _headers: &hyper::HeaderMap<hyper::header::HeaderValue>,
    Path(linking_id): Path<String>,
    Extension(with_rsvp_scope): Extension<WithRsvpScope>,
) -> Result<MessageTemplate, WebError> {
    let mut redis_connection = state
        .redis_client
        .get_multiplexed_async_connection()
        .await?;
    // The linking ID was stored in Redis when the linking link was created.
    // Check that it is still valid
    let redis_key = format!("meetup_linking:{}:discord_user", &linking_id);
    // This is a one-time use link. Expire it now.
    let mut pipe = redis::pipe();
    pipe.get(&redis_key).del(&redis_key);
    let (discord_id, _): (Option<u64>, u32) = pipe.query_async(&mut redis_connection).await?;
    let discord_id = match discord_id {
        Some(id) => UserId::new(id),
        None => {
            return Ok((
                lib::strings::OAUTH2_LINK_EXPIRED_TITLE,
                lib::strings::OAUTH2_LINK_EXPIRED_CONTENT,
            )
                .into())
        }
    };
    if let Some(error) = query.error {
        if error == "access_denied" {
            // The user did not grant access
            // Give them the chance to do it again
            let linking_url = lib::meetup::oauth2::generate_meetup_linking_link(
                &mut redis_connection,
                discord_id,
            )
            .await?;
            return Ok(MessageTemplate {
                title: Cow::Borrowed("Linking Failure"),
                content: None,
                img_url: None,
                safe_content: Some(Cow::Owned(lib::strings::OAUTH2_AUTHORISATION_DENIED(
                    &linking_url,
                ))),
            });
        } else {
            // Some other error occured
            eprintln!("Received an OAuth2 error code from Meetup: {}", error);
            return Ok(("OAuth2 error", error.to_string()).into());
        }
    }
    // Compare the CSRF state that was returned by Meetup to the one
    // we have saved
    // let csrf_is_valid = check_csrf_cookie(redis_connection, headers, &query.state).await?;
    // if !csrf_is_valid {
    //     return Ok((
    //         "CSRF check failed",
    //         "Please go back to the first page, reload, and repeat the process",
    //     )
    //         .into());
    // }
    // Exchange the code with a token.
    let code = AuthorizationCode::new(query.code);
    let redirect_url = RedirectUrl::new(format!("{}{}", lib::urls::BASE_URL, path.path()))?;
    let token_res = (*state.oauth2_consumer.link_client)
        .clone()
        .set_redirect_uri(redirect_url)
        .exchange_code(code)
        .request_async(oauth2::reqwest::async_http_client)
        .await?;
    // Get the user's Meetup ID
    let async_user_meetup_client =
        lib::meetup::newapi::AsyncClient::new(token_res.access_token().secret());
    let meetup_user = async_user_meetup_client.get_self().await?;
    // Link the Discord and Meetup profiles
    let mut tx = state.pool.begin().await?;
    let linking_result = lib::link_discord_meetup(discord_id, meetup_user.id.0, &mut tx).await?;
    tx.commit().await?;

    match linking_result {
        LinkingResult::Success {
            action: LinkingAction::AlreadyLinked,
            ..
        } => Ok((
            lib::strings::OAUTH2_ALREADY_LINKED_SUCCESS_TITLE,
            lib::strings::OAUTH2_ALREADY_LINKED_SUCCESS_CONTENT,
        )
            .into()),
        LinkingResult::Success {
            action: LinkingAction::Linked | LinkingAction::NewMember | LinkingAction::MergedMember,
            member_id,
        } => {
            // If the "rsvp" scope is part of the token result, store the tokens as well
            if with_rsvp_scope.0 {
                if let Some(refresh_token) = token_res.refresh_token() {
                    sqlx::query!(
                        r#"UPDATE "member" SET meetup_oauth2_access_token = $2, meetup_oauth2_refresh_token = $3 WHERE id = $1"#,
                        member_id.0,
                        token_res.access_token().secret(),
                        refresh_token.secret()
                    ).execute(&state.pool).await.ok();
                }
            }
            if let Some(photo_url) = meetup_user
                .member_photo
                .and_then(|photo| photo.url_for_size(380, 380))
            {
                Ok(MessageTemplate {
                    title: Cow::Borrowed(lib::strings::OAUTH2_LINKING_SUCCESS_TITLE),
                    content: Some(Cow::Owned(lib::strings::OAUTH2_LINKING_SUCCESS_CONTENT(
                        meetup_user.name.unwrap_or_str("Unknown"),
                    ))),
                    safe_content: None,
                    img_url: Some(Cow::Owned(photo_url)),
                }
                .into())
            } else {
                Ok((
                    lib::strings::OAUTH2_LINKING_SUCCESS_TITLE,
                    lib::strings::OAUTH2_LINKING_SUCCESS_CONTENT(
                        meetup_user.name.unwrap_or_str("Unknown"),
                    ),
                )
                    .into())
            }
        }
        LinkingResult::Conflict {
            member_with_meetup:
                LinkingMemberMeetup {
                    meetup_id: _meetup_id1,
                    discord_id: discord_id1,
                    ..
                },
            member_with_discord:
                LinkingMemberDiscord {
                    meetup_id: meetup_id2,
                    discord_id: _discord_id2,
                    ..
                },
        } => {
            if let Some(_discord_id1) = discord_id1 {
                Ok((
                    lib::strings::OAUTH2_DISCORD_ALREADY_LINKED_FAILURE_TITLE,
                    lib::strings::OAUTH2_DISCORD_ALREADY_LINKED_FAILURE_CONTENT(&state.bot_name),
                )
                    .into())
            } else if let Some(_meetup_id2) = meetup_id2 {
                Ok((
                    lib::strings::OAUTH2_MEETUP_ALREADY_LINKED_FAILURE_TITLE,
                    lib::strings::OAUTH2_MEETUP_ALREADY_LINKED_FAILURE_CONTENT(&state.bot_name),
                )
                    .into())
            } else {
                Ok((
                    "Linking Failure",
                    "Could not assign meetup id (timing error)",
                )
                    .into())
            }
        }
    }
}
