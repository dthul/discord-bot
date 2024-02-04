use oauth2::{
    basic::BasicClient, AuthUrl, ClientId, ClientSecret, RedirectUrl, TokenResponse, TokenUrl,
};
use redis::AsyncCommands;
use serenity::model::id::UserId;
use simple_error::SimpleError;
use std::sync::Arc;

use crate::db;

// TODO: move into flow?
#[tracing::instrument(skip(redis_connection))]
pub async fn generate_meetup_linking_link(
    redis_connection: &mut redis::aio::Connection,
    discord_id: UserId,
) -> Result<String, super::Error> {
    let linking_id = crate::new_random_id(16);
    // TODO: expire after a day or so? linking.rs adds a 10 min expiration after first opening
    let redis_key = format!("meetup_linking:{}:discord_user", &linking_id);
    if let Err::<(), _>(err) = redis_connection.set(&redis_key, discord_id.get()).await {
        return Err(SimpleError::new(format!(
            "Redis error when trying to generate Meetup linking link:\n{:#?}",
            err
        ))
        .into());
    }
    return Ok(format!("{}/link/{}", crate::urls::BASE_URL, &linking_id));
}

#[derive(Clone)]
pub struct OAuth2Consumer {
    pub authorization_client: Arc<BasicClient>,
    pub link_client: Arc<BasicClient>,
}

impl OAuth2Consumer {
    pub fn new(
        meetup_client_id: String,
        meetup_client_secret: String,
    ) -> Result<Self, crate::BoxedError> {
        let meetup_client_id = ClientId::new(meetup_client_id);
        let meetup_client_secret = ClientSecret::new(meetup_client_secret);
        let auth_url = AuthUrl::new(crate::urls::MEETUP_OAUTH2_AUTH_URL.to_string())?;
        let token_url = TokenUrl::new(crate::urls::MEETUP_OAUTH2_TOKEN_URL.to_string())?;

        // Set up the config for the Github OAuth2 process.
        let authorization_client = BasicClient::new(
            meetup_client_id,
            Some(meetup_client_secret),
            auth_url,
            Some(token_url),
        )
        .set_auth_type(oauth2::AuthType::RequestBody)
        .set_redirect_uri(RedirectUrl::new(format!(
            "{}/authorize/redirect",
            crate::urls::BASE_URL
        ))?);
        let link_client = authorization_client
            .clone()
            .set_redirect_uri(RedirectUrl::new(format!(
                "{}/link/redirect",
                crate::urls::BASE_URL
            ))?);
        let authorization_client = Arc::new(authorization_client);
        let link_client = Arc::new(link_client);

        Ok(OAuth2Consumer {
            authorization_client: authorization_client,
            link_client: link_client,
        })
    }

    pub async fn refresh_oauth_tokens(
        &self,
        token_type: TokenType,
        db_connection: &sqlx::PgPool,
    ) -> Result<oauth2::AccessToken, super::Error> {
        refresh_oauth_tokens(token_type, &self.authorization_client, db_connection).await
    }
}

#[derive(Debug, Clone, Copy)]
pub enum TokenType {
    Member(db::MemberId),
    Organizer,
}

#[tracing::instrument(skip(oauth2_client, db_connection))]
pub async fn refresh_oauth_tokens(
    token_type: TokenType,
    oauth2_client: &BasicClient,
    db_connection: &sqlx::PgPool,
) -> Result<oauth2::AccessToken, super::Error> {
    // Try to get the refresh token from the database and lock the row
    let mut tx = db_connection.begin().await?;
    let refresh_token: Option<String> = match token_type {
        TokenType::Organizer => {
            sqlx::query_scalar!(r#"SELECT meetup_refresh_token FROM organizer_token FOR UPDATE"#)
                .fetch_optional(&mut *tx)
                .await?
                .flatten()
        }
        TokenType::Member(member_id) => sqlx::query_scalar!(
            r#"SELECT meetup_oauth2_refresh_token FROM "member" WHERE id = $1 FOR UPDATE"#,
            member_id.0
        )
        .fetch_optional(&mut *tx)
        .await?
        .flatten(),
    };
    let refresh_token: String = match refresh_token {
        Some(refresh_token) => refresh_token,
        None => {
            return Err(SimpleError::new(
                "Could not refresh the Meetup access token since there is no refresh token \
                 available",
            )
            .into())
        }
    };
    // Try to exchange the refresh token for fresh access and refresh tokens
    let refresh_token = oauth2::RefreshToken::new(refresh_token);
    let refresh_token_response = oauth2_client
        .exchange_refresh_token(&refresh_token)
        .request_async(oauth2::reqwest::async_http_client)
        .await?;
    // Store the new tokens
    match token_type {
        TokenType::Organizer => {
            sqlx::query!(r#"DELETE FROM organizer_token"#)
                .execute(&mut *tx)
                .await?;
            sqlx::query!(
                r#"INSERT INTO organizer_token (meetup_access_token, meetup_refresh_token, meetup_access_token_refresh_time) VALUES ($1, $2, $3)"#,
                refresh_token_response.access_token().secret(),
                refresh_token_response.refresh_token().map(|token| token.secret()),
                chrono::Utc::now() + chrono::Duration::days(2))
                .execute(&mut *tx)
                .await?;
        }
        TokenType::Member(member_id) => {
            sqlx::query!(
                r#"UPDATE "member" SET meetup_oauth2_access_token = $2, meetup_oauth2_refresh_token = $3, meetup_oauth2_last_token_refresh_time = $4 WHERE id = $1"#,
                member_id.0,
                refresh_token_response.access_token().secret(),
                refresh_token_response
                    .refresh_token()
                    .map(|token| token.secret()),
                chrono::Utc::now()
            ).execute(&mut *tx).await?;
        }
    };
    tx.commit().await?;
    Ok(refresh_token_response.access_token().clone())
}
