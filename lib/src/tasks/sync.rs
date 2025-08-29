use futures_util::lock::Mutex;
use serenity::model::id::UserId;
use std::sync::Arc;
use tokio::time::{Duration, Instant};

use crate::{free_spots::EventCollector, swissrpg::client::SwissRPGClient};

pub async fn create_recurring_syncing_task(
    db_connection: sqlx::PgPool,
    redis_client: redis::Client,
    swissrpg_client: Arc<SwissRPGClient>,
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
        let swissrpg_client = swissrpg_client.clone();
        let meetup_client = meetup_client.clone();
        tokio::spawn(async move {
            let mut redis_connection = redis_client.get_multiplexed_async_connection().await?;
            let mut event_collector = EventCollector::new();
            // Sync with Meetup
            match tokio::time::timeout(
                Duration::from_secs(360),
                crate::meetup::sync::sync_task(meetup_client.clone(), &db_connection),
            )
            .await
            {
                Err(_) => eprintln!("Meetup syncing task timed out"),
                Ok(sync_result) => {
                    match sync_result {
                        Ok(meetup_event_collector) => {
                            // Merge events from Meetup
                            for event in meetup_event_collector.events {
                                event_collector.add_event(event);
                            }
                        }
                        Err(err) => eprintln!("Meetup syncing task failed: {}", err),
                    }
                }
            };
            // Sync with SwissRPG
            match tokio::time::timeout(
                Duration::from_secs(360),
                crate::swissrpg::sync::sync_task(swissrpg_client.clone(), &db_connection),
            )
            .await
            {
                Err(_) => eprintln!("SwissRPG syncing task timed out"),
                Ok(sync_result) => {
                    match sync_result {
                        Ok(swissrpg_event_collector) => {
                            // Merge events from SwissRPG
                            for event in swissrpg_event_collector.events {
                                event_collector.add_event(event);
                            }
                        }
                        Err(err) => eprintln!("SwissRPG syncing task failed: {}", err),
                    }
                }
            };
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
