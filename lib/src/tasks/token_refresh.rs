use crate::{db, meetup::oauth2::TokenType};
use chrono::{Datelike, Timelike};
use futures_util::lock::Mutex;
use std::sync::Arc;

// Refreshes the authorization token
pub async fn organizer_token_refresh_task(
    oauth2_consumer: crate::meetup::oauth2::OAuth2Consumer,
    pool: sqlx::PgPool,
    async_meetup_client: Arc<Mutex<Option<Arc<crate::meetup::newapi::AsyncClient>>>>,
) -> ! {
    // Try to get the next scheduled refresh time from the database, otherwise
    // schedule a refresh immediately
    let next_refresh_time =
        sqlx::query_scalar!(r#"SELECT meetup_access_token_refresh_time FROM organizer_token"#)
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten()
            .flatten();
    let mut next_refresh_time = if let Some(next_refresh_time) = next_refresh_time {
        next_refresh_time
    } else {
        chrono::Utc::now()
    };
    loop {
        println!(
            "Next organizer token refresh @ {}",
            next_refresh_time.to_rfc3339()
        );
        let wait_duration_in_secs = (next_refresh_time - chrono::Utc::now()).num_seconds();
        if wait_duration_in_secs > 0 {
            tokio::time::sleep(tokio::time::Duration::from_secs(
                wait_duration_in_secs as u64,
            ))
            .await;
        }
        println!("Starting organizer token refresh");
        // Try to refresh the organizer oauth tokens.
        // We spawn this onto a new task, such that when this long-lived refresh task
        // is aborted, the short-lived refresh task still has a chance to run to completion.
        let join_handle = {
            let oauth2_consumer = oauth2_consumer.clone();
            let pool = pool.clone();
            let async_meetup_client = async_meetup_client.clone();
            tokio::spawn(async move {
                organizer_token_refresh_task_impl(oauth2_consumer, pool, async_meetup_client).await
            })
        };
        match join_handle.await {
            Err(err) => {
                eprintln!("Could not refresh the organizer's oauth2 token:\n{}\n", err);
                // Try to refresh again in an hour
                next_refresh_time = chrono::Utc::now() + chrono::Duration::hours(1);
            }
            Ok(Err(err)) => {
                eprintln!("Could not refresh the organizer's oauth2 token:\n{}\n", err);
                // Try to refresh again in an hour
                next_refresh_time = chrono::Utc::now() + chrono::Duration::hours(1);
            }
            Ok(Ok(())) => {
                // Refresh the access token in two days from now
                next_refresh_time = chrono::Utc::now() + chrono::Duration::days(2);
                println!(
                    "Refreshed the organizer's Meetup OAuth token. Next refresh @ {}",
                    next_refresh_time.to_rfc3339()
                );
                // Store refresh date in the database, ignore failures
                sqlx::query!(
                    r#"UPDATE organizer_token SET meetup_access_token_refresh_time = $1"#,
                    next_refresh_time
                )
                .execute(&pool)
                .await
                .ok();
            }
        }
    }
}

async fn organizer_token_refresh_task_impl(
    oauth2_consumer: crate::meetup::oauth2::OAuth2Consumer,
    pool: sqlx::PgPool,
    async_meetup_client: Arc<Mutex<Option<Arc<crate::meetup::newapi::AsyncClient>>>>,
) -> Result<(), crate::meetup::Error> {
    // Try to refresh the organizer oauth tokens
    let new_auth_token = crate::meetup::oauth2::refresh_oauth_tokens(
        TokenType::Organizer,
        &oauth2_consumer.authorization_client,
        &oauth2_consumer.http_client,
        &pool,
    )
    .await?;
    let mut async_meetup_guard = async_meetup_client.lock().await;
    *async_meetup_guard = Some(Arc::new(crate::meetup::newapi::AsyncClient::new(
        new_auth_token.secret(),
    )));
    drop(async_meetup_guard);
    Ok(())
}

pub async fn users_token_refresh_task(
    oauth2_consumer: crate::meetup::oauth2::OAuth2Consumer,
    pool: sqlx::PgPool,
) -> ! {
    let mut interval_timer = tokio::time::interval_at(
        tokio::time::Instant::now() + tokio::time::Duration::from_secs(60),
        tokio::time::Duration::from_secs(30 * 60),
    );
    // Run forever
    loop {
        // Wait for the next interval tick
        interval_timer.tick().await;
        // Poor man's try block
        let res: Result<_, crate::meetup::Error> = (|| async {
            let member_ids = sqlx::query!(r#"SELECT id FROM "member""#)
                .map(|row| db::MemberId(row.id))
                .fetch_all(&pool)
                .await?;
            // For each user, check if there is a refresh token.
            // If so, check whether refresh is due.
            println!(
                "Users token refresh task: Checking {} users",
                member_ids.len()
            );
            for member_id in member_ids {
                // Try to refresh the user's oauth tokens.
                // We spawn this onto a new task, such that when this long-lived refresh task
                // is aborted, the short-lived refresh task still has a chance to run to completion.
                let join_handle = {
                    let oauth2_consumer = oauth2_consumer.clone();
                    let pool = pool.clone();
                    tokio::spawn(async move {
                        user_token_refresh_task_impl(member_id, oauth2_consumer, pool).await
                    })
                };
                match join_handle.await {
                    Err(err) => {
                        eprintln!("Could not refresh the user's oauth2 token:\n{:#?}\n", err);
                    }
                    Ok(Err(err)) => {
                        eprintln!("Could not refresh the user's oauth2 token:\n{:#?}\n", err);
                    }
                    Ok(Ok(())) => {
                        // Nothing to do
                    }
                }
                // Just to make sure that we are really interruptible
                tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
            }
            Ok(())
        })()
        .await;
        if let Err(err) = res {
            eprintln!("Error in users refresh token task:\n{:#?}", err);
        }
    }
}

async fn user_token_refresh_task_impl(
    member_id: db::MemberId,
    oauth2_consumer: crate::meetup::oauth2::OAuth2Consumer,
    pool: sqlx::PgPool,
) -> Result<(), crate::meetup::Error> {
    // Check if the user has a refresh token
    let has_refresh_token =
        sqlx::query_scalar!(r#"SELECT COUNT(*) > 0 AS "has_refresh_token!" FROM "member" WHERE id = $1 AND meetup_oauth2_refresh_token IS NOT NULL"#, member_id.0)
            .fetch_one(&pool)
            .await?;
    if !has_refresh_token {
        // Nothing to do
        return Ok(());
    }
    // Check whether the user's token is due for refresh
    let last_refresh_time = sqlx::query_scalar!(
        r#"SELECT meetup_oauth2_last_token_refresh_time FROM "member" WHERE id = $1"#,
        member_id.0 as i64
    )
    .fetch_one(&pool)
    .await?;
    if let Some(last_refresh_time) = last_refresh_time {
        // If the last refresh has been more recently than a month, skip it
        if last_refresh_time + chrono::Duration::days(30) > chrono::Utc::now() {
            return Ok(());
        }
    } else {
        // If the token has not been refreshed yet, check whether now is a
        // good time to do so.
        let now = chrono::Utc::now();
        let day_number = now.num_days_from_ce();
        let hour_number = now.hour(); // [0, 23]
        let bucket_number = (day_number % 4) as u32 * 24 + hour_number; // [0, 95]
        if (member_id.0 % 96) as u32 != bucket_number {
            // Now is not a good time
            return Ok(());
        }
    }
    // Try to refresh the user's oauth tokens
    println!("Refreshing oauth2 token of member {}", member_id.0);
    let new_auth_token = crate::meetup::oauth2::refresh_oauth_tokens(
        TokenType::Member(member_id),
        &oauth2_consumer.authorization_client,
        &oauth2_consumer.http_client,
        &pool,
    )
    .await;
    match new_auth_token {
        Ok(_) => {
            println!(
                "OAuth2 tokens of member {} successfully refreshed",
                member_id.0
            );
        }
        Err(err) => {
            return Err(err);
        }
    }
    Ok(())
}
