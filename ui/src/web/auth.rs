use std::{borrow::Cow, sync::Arc};

use askama::Template;
use axum::{
    body::Body,
    extract::{Extension, Form, Path},
    http::{header::SET_COOKIE, HeaderValue, Request},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use axum_extra::headers::HeaderMapExt;
use axum_extra::TypedHeader;
use base64::{engine::general_purpose, Engine as _};
use cookie::{Cookie, CookieJar, Key, SameSite};
use lib::db::MemberId;
use redis::AsyncCommands;
use serde::Deserialize;
use serenity::{builder::CreateMessage, model::id::UserId};
use simple_error::SimpleError;

use super::{server::State, MessageTemplate, WebError};

pub fn create_routes() -> Router {
    Router::new()
        .route("/login/:auth_id", get(auth_handler_get))
        .route("/login", post(auth_handler_post))
        .route("/logout", post(logout_handler))
}
// In Redis: session ID, member ID and last used time
// Routes protected by auth:
// - is there a session ID (signed or secret) cookie? Is the session ID in Redis and is the last used time not too long ago?
//   Then allow access for that Member (Member should probably be a return type for auth)
//   Update the last used time to "now"
// - no cookie or last used time too far in the past? Delete the session from Redis, delete the cookie and show login instructions (get link from Hyperion)
// - possibly in the future: require 2FA for admins (like TOTP) for first login and if the last used time is older than a certain threshold (but not so old that it would count as expired)

pub async fn generate_login_link(
    redis_connection: &mut redis::aio::MultiplexedConnection,
    discord_id: UserId,
) -> Result<String, lib::meetup::Error> {
    let auth_id = lib::new_random_id(16);
    let redis_key = format!("web_session_auth:{}:discord_user", auth_id);
    let mut pipe = redis::pipe();
    let _: () = pipe
        .set(&redis_key, discord_id.get())
        .ignore()
        .expire(&redis_key, 10 * 60)
        .query_async(redis_connection)
        .await?;
    return Ok(format!("{}/login/{}", lib::urls::BASE_URL, auth_id));
}

const AUTH_COOKIE_NAME: &'static str = "__Host-Hyperion-Session-Id";

#[derive(Deserialize)]
struct AuthForm {
    auth_id: String, // base64 encoded
}

#[derive(Template)]
#[template(path = "login.html")]
struct AuthTemplate<'a> {
    auth_id: &'a str,
}

#[derive(Debug, Copy, Clone)]
pub struct AuthenticatedMember(pub MemberId);

async fn auth_handler_get(
    Path(auth_id): Path<String>,
    state: Extension<Arc<State>>,
) -> Result<Response, WebError> {
    let mut redis_connection = state
        .redis_client
        .get_multiplexed_async_connection()
        .await?;
    let redis_key = format!("web_session_auth:{}:discord_user", &auth_id);
    // Check if this auth ID is valid
    let discord_id: Option<u64> = redis_connection.get(redis_key).await?;
    if discord_id.is_none() {
        let template: MessageTemplate = (
            "This link seems to have expired",
            "Get a new link with the \"login\" command",
        )
            .into();
        return Ok(template.into_response());
    }
    // Show the login form
    let template = AuthTemplate { auth_id: &auth_id };
    Ok(template.into_response())
}

async fn auth_handler_post(
    state: Extension<Arc<State>>,
    form: Form<AuthForm>,
) -> Result<Response, WebError> {
    let mut redis_connection = state
        .redis_client
        .get_multiplexed_async_connection()
        .await?;
    let redis_key = format!("web_session_auth:{}:discord_user", form.auth_id);
    // This is a one-time use link. Expire it now.
    let mut pipe = redis::pipe();
    pipe.get(&redis_key).del(&redis_key);
    let (discord_id, _): (Option<u64>, u32) = pipe.query_async(&mut redis_connection).await?;
    let discord_id = match discord_id {
        Some(id) => UserId::new(id),
        None => {
            let template: MessageTemplate = (
                "This link seems to have expired",
                "Get a new link with the \"login\" command",
            )
                .into();
            return Ok(template.into_response());
        }
    };
    // Get the member corresponding to the Discord ID
    let mut tx = state.pool.begin().await?;
    let member_id = lib::db::get_or_create_member_for_discord_id(&mut tx, discord_id).await?;
    // Store a new web session in the database
    let session_id = lib::new_random_id_raw(16);
    sqlx::query!(
        r#"INSERT INTO web_session (session_id, member_id) VALUES ($1, $2)"#,
        &session_id,
        member_id.0
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let session_id_encoded = general_purpose::URL_SAFE_NO_PAD.encode(session_id);
    let auth_cookie = Cookie::build((AUTH_COOKIE_NAME, session_id_encoded))
        .same_site(SameSite::Strict)
        .secure(true)
        .http_only(true)
        // .max_age(cookie::time::Duration::days(2))
        .path("/");
    let key = get_or_create_cookie_key(&*state).await?;
    let mut jar = CookieJar::new();
    jar.private_mut(&key).add(auth_cookie);
    // let auth_cookie_header = HeaderValue::from_str(&auth_cookie.to_string())?;
    let mut response = Redirect::to("/").into_response();
    for cookie_to_set in jar.delta() {
        let cookie_header_value: HeaderValue = cookie_to_set.to_string().parse()?;
        response
            .headers_mut()
            .insert(SET_COOKIE, cookie_header_value);
    }
    // TODO: when merging member, also merge the web sessions (or remove them)
    // Notify on Discord about new login
    // TODO: maybe only for admins
    if let Ok(user) = discord_id.to_user(&state.discord_cache_http).await {
        user.id
            .direct_message(
                &state.discord_cache_http.http,
                CreateMessage::new().content("New web login registered"),
            )
            .await
            .ok();
    }
    Ok(response)
}

async fn get_or_create_cookie_key(state: &State) -> Result<Key, WebError> {
    // Get cookie key from the database or generate a new one if there is no key
    let mut tx = state.pool.begin().await?;
    let key = sqlx::query_scalar!(r#"SELECT cookie_key FROM ephemeral_settings"#)
        .fetch_optional(&mut *tx)
        .await?
        .flatten();
    let key = match key {
        Some(key) if key.len() >= 64 => Key::from(&key),
        _ => match Key::try_generate() {
            None => return Err(SimpleError::new("Could not generate a cookie key").into()),
            Some(key) => {
                sqlx::query!(
                    r#"INSERT INTO ephemeral_settings (cookie_key)
                        VALUES ($1)
                        ON CONFLICT (id) DO UPDATE
                        SET cookie_key = $1"#,
                    key.master()
                )
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                key
            }
        },
    };
    Ok(key)
}

struct RemoveAuthCookie<T: IntoResponse>(pub T);

impl<T: IntoResponse> IntoResponse for RemoveAuthCookie<T> {
    fn into_response(self) -> Response {
        let deletion_cookie = Cookie::build((AUTH_COOKIE_NAME, ""))
            .same_site(SameSite::Strict)
            .secure(true)
            .http_only(true)
            .path("/")
            .expires(cookie::time::OffsetDateTime::now_utc() - cookie::time::Duration::weeks(50));
        let mut response = self.0.into_response();
        if let Ok(deletion_cookie_value) = deletion_cookie.to_string().parse::<HeaderValue>() {
            response
                .headers_mut()
                .insert(SET_COOKIE, deletion_cookie_value);
        }
        response
    }
}

async fn logout_handler(
    TypedHeader(cookie_header): TypedHeader<axum_extra::headers::Cookie>,
    Extension(state): Extension<Arc<State>>,
) -> Result<impl IntoResponse, WebError> {
    // Check if there is an auth cookie with a valid session ID
    let key = get_or_create_cookie_key(&*state).await?;
    let mut jar = CookieJar::new();
    for (cookie_name, cookie_value) in cookie_header.iter() {
        jar.add_original(Cookie::new(cookie_name, cookie_value).into_owned());
    }
    let auth_cookie = match jar.private(&key).get(AUTH_COOKIE_NAME) {
        None => return Err(WebError::Unauthorized(None)),
        Some(cookie) => cookie,
    };
    let session_id_encoded = auth_cookie.value_trimmed();
    let session_id = general_purpose::URL_SAFE_NO_PAD
        .decode(session_id_encoded)
        .map_err(|_| WebError::Unauthorized(None))?;
    sqlx::query!(
        r#"DELETE FROM web_session WHERE session_id = $1"#,
        &session_id
    )
    .execute(&state.pool)
    .await
    .ok();
    Ok(RemoveAuthCookie(Redirect::to("/")))
}

pub async fn auth(mut req: Request<Body>, next: Next) -> Result<Response, WebError> {
    let state: &Arc<State> = match req.extensions().get() {
        Some(state) => state,
        None => return Err(SimpleError::new("State is not set").into()),
    };
    let key = get_or_create_cookie_key(&*state).await?;
    // It looks like typed_get() (and the TypedHeader extractor) will merge all
    // occurences of a specific header, so this should be sufficient to handle
    // multiple "Cookie" headers (which is allowed by HTTP2)
    let cookie_header: Option<axum_extra::headers::Cookie> = req.headers().typed_get();
    let mut jar = CookieJar::new();
    if let Some(cookie_header) = cookie_header {
        for (cookie_name, cookie_value) in cookie_header.iter() {
            jar.add_original(Cookie::new(cookie_name, cookie_value).into_owned());
        }
    }
    let auth_cookie = match jar.private(&key).get(AUTH_COOKIE_NAME) {
        None => return Err(WebError::Unauthorized(None)),
        Some(cookie) => cookie,
    };
    let session_id_encoded = auth_cookie.value_trimmed();
    let session_id = general_purpose::URL_SAFE_NO_PAD
        .decode(session_id_encoded)
        .map_err(|_| WebError::Unauthorized(None))?;
    let (session_db_id, member_id, last_used) = match sqlx::query!(
        r#"SELECT id, member_id, last_used FROM web_session WHERE session_id = $1"#,
        &session_id
    )
    .map(|row| (row.id, MemberId(row.member_id), row.last_used))
    .fetch_optional(&state.pool)
    .await?
    {
        Some(row) => row,
        None => return Ok(RemoveAuthCookie(WebError::Unauthorized(None)).into_response()),
    };
    if last_used + chrono::Duration::days(2) < chrono::Utc::now() {
        // Expired
        sqlx::query!(r#"DELETE FROM web_session WHERE id = $1"#, session_db_id)
            .execute(&state.pool)
            .await
            .ok();
        return Ok(RemoveAuthCookie(WebError::Unauthorized(Some(Cow::Borrowed(
            "Session expired",
        ))))
        .into_response());
    }
    if last_used + chrono::Duration::hours(1) < chrono::Utc::now() {
        // Update last used time
        sqlx::query!(
            r#"UPDATE web_session SET last_used = NOW() WHERE id = $1"#,
            session_db_id
        )
        .execute(&state.pool)
        .await
        .ok();
    }
    req.extensions_mut().insert(AuthenticatedMember(member_id));
    Ok(next.run(req).await)
    // Since this is a middleware we have the option of adjusting the response here (e.g. adding Set-Cookie headers)
}
