use futures::future::TryFutureExt;
use futures_util::lock::Mutex;
use serenity::model::id::UserId;
use std::sync::Arc;
use tokio::time::{Duration, Instant};

pub async fn create_recurring_syncing_task(
    db_connection: sqlx::PgPool,
    redis_client: redis::Client,
    meetup_client: Arc<Mutex<Option<Arc<crate::meetup::newapi::AsyncClient>>>>,
    discord_api: crate::discord::CacheAndHttp,
    bot_id: UserId,
    static_file_prefix: &'static str,
) -> ! {
    let mut interval_timer = tokio::time::interval_at(
        Instant::now() + Duration::from_secs(15 * 60),
        Duration::from_secs(15 * 60),
    );
    // Run forever
    loop {
        // Wait for the next interval tick
        interval_timer.tick().await;
        let db_connection = db_connection.clone();
        let redis_client = redis_client.clone();
        let discord_api = discord_api.clone();
        let meetup_client = meetup_client.clone();
        tokio::spawn(async move {
            let mut redis_connection = redis_client.get_multiplexed_async_connection().await?;
            // Sync with Meetup
            let event_collector = tokio::time::timeout(
                Duration::from_secs(360),
                crate::meetup::sync::sync_task(meetup_client.clone(), &db_connection).map_err(
                    |err| {
                        eprintln!("Syncing task failed: {}", err);
                        err
                    },
                ),
            )
            .map_err(|err| {
                eprintln!("Syncing task timed out: {}", err);
                err
            })
            .await??;
            // Sync with Discord
            if let Err(err) = crate::discord::sync::sync_discord(
                &mut redis_connection,
                &db_connection,
                &discord_api,
                bot_id,
            )
            .await
            {
                eprintln!("Discord syncing task failed: {}", err);
            }
            // Finally, update Discord with the information on open spots.
            if let Some(channel_id) = crate::discord::sync::ids::FREE_SPOTS_CHANNEL_ID {
                if let Err(err) = event_collector
                    .update_channel(&discord_api, channel_id, static_file_prefix)
                    .await
                {
                    eprintln!("Error when posting open game spots:\n{:#?}", err);
                }
            } else {
                eprintln!("No channel configured for posting open game spots");
            }
            if let Err(err) = event_collector
                .assign_roles(meetup_client.clone(), &db_connection, &discord_api)
                .await
            {
                eprintln!("Error in EventCollector::assign_roles:\n{:#?}", err);
            }
            Ok::<_, crate::BoxedError>(())
        });
    }
}
