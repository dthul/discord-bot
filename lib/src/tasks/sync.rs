use futures_util::lock::Mutex;
use serenity::model::id::UserId;
use std::sync::Arc;
use tokio::time::{Duration, Instant};

use crate::{free_spots::EventCollector, swissrpg::client::SwissRPGClient};

/// Shared state for the latest event collectors from each sync task
#[derive(Debug, Clone, Default)]
pub struct SyncState {
    pub meetup_events: Arc<Mutex<Option<EventCollector>>>,
    pub swissrpg_events: Arc<Mutex<Option<EventCollector>>>,
}

/// Independent Meetup sync task
#[tracing::instrument(skip(db_connection, meetup_client, sync_state))]
pub async fn create_recurring_meetup_sync_task(
    db_connection: sqlx::PgPool,
    meetup_client: Arc<Mutex<Option<Arc<crate::meetup::newapi::AsyncClient>>>>,
    sync_state: SyncState,
) {
    let mut interval_timer = tokio::time::interval_at(
        Instant::now() + Duration::from_secs(15 * 60),
        Duration::from_secs(15 * 60),
    );
    
    loop {
        interval_timer.tick().await;
        
        let db_connection = db_connection.clone();
        let meetup_client = meetup_client.clone();
        let sync_state = sync_state.clone();
        
        tokio::spawn(async move {
            match tokio::time::timeout(
                Duration::from_secs(360),
                crate::meetup::sync::sync_task(meetup_client, &db_connection),
            )
            .await
            {
                Err(_) => eprintln!("Meetup syncing task timed out"),
                Ok(sync_result) => {
                    match sync_result {
                        Ok(event_collector) => {
                            // Update shared state with latest Meetup events
                            *sync_state.meetup_events.lock().await = Some(event_collector);
                            println!("Meetup sync completed successfully");
                        }
                        Err(err) => eprintln!("Meetup syncing task failed: {}", err),
                    }
                }
            }
        });
    }
}

/// Independent SwissRPG sync task
#[tracing::instrument(skip(db_connection, swissrpg_client, sync_state))]
pub async fn create_recurring_swissrpg_sync_task(
    db_connection: sqlx::PgPool,
    swissrpg_client: Arc<SwissRPGClient>,
    sync_state: SyncState,
) {
    let mut interval_timer = tokio::time::interval_at(
        Instant::now() + Duration::from_secs(20 * 60), // Offset by 5 minutes from Meetup
        Duration::from_secs(15 * 60),
    );
    
    loop {
        interval_timer.tick().await;
        
        let db_connection = db_connection.clone();
        let swissrpg_client = swissrpg_client.clone();
        let sync_state = sync_state.clone();
        
        tokio::spawn(async move {
            match tokio::time::timeout(
                Duration::from_secs(360),
                crate::swissrpg::sync::sync_task(swissrpg_client, &db_connection),
            )
            .await
            {
                Err(_) => eprintln!("SwissRPG syncing task timed out"),
                Ok(sync_result) => {
                    match sync_result {
                        Ok(event_collector) => {
                            // Update shared state with latest SwissRPG events
                            *sync_state.swissrpg_events.lock().await = Some(event_collector);
                            println!("SwissRPG sync completed successfully");
                        }
                        Err(err) => eprintln!("SwissRPG syncing task failed: {}", err),
                    }
                }
            }
        });
    }
}

/// Discord sync task (for channels, roles, etc.)
#[tracing::instrument(skip(db_connection, redis_client, discord_api))]
pub async fn create_recurring_discord_sync_task(
    db_connection: sqlx::PgPool,
    redis_client: redis::Client,
    discord_api: crate::discord::CacheAndHttp,
    bot_id: UserId,
) {
    let mut interval_timer = tokio::time::interval_at(
        Instant::now() + Duration::from_secs(10 * 60), // Offset by 10 minutes
        Duration::from_secs(15 * 60),
    );
    
    loop {
        interval_timer.tick().await;
        
        let db_connection = db_connection.clone();
        let redis_client = redis_client.clone();
        let discord_api = discord_api.clone();
        
        tokio::spawn(async move {
            let mut redis_connection = match redis_client.get_multiplexed_async_connection().await {
                Ok(conn) => conn,
                Err(err) => {
                    eprintln!("Failed to get Redis connection for Discord sync: {}", err);
                    return;
                }
            };
            
            if let Err(err) = crate::discord::sync::sync_discord(
                &mut redis_connection,
                &db_connection,
                &discord_api,
                bot_id,
            )
            .await
            {
                eprintln!("Discord syncing task failed: {}", err);
            } else {
                println!("Discord sync completed successfully");
            }
        });
    }
}

/// Free spots task that combines events from both sources
#[tracing::instrument(skip(sync_state, meetup_client, discord_api))]
pub async fn create_recurring_free_spots_task(
    sync_state: SyncState,
    meetup_client: Arc<Mutex<Option<Arc<crate::meetup::newapi::AsyncClient>>>>,
    db_connection: sqlx::PgPool,
    discord_api: crate::discord::CacheAndHttp,
    static_file_prefix: &'static str,
) {
    let mut interval_timer = tokio::time::interval_at(
        Instant::now() + Duration::from_secs(25 * 60), // Run after both syncs likely completed
        Duration::from_secs(15 * 60),
    );
    
    loop {
        interval_timer.tick().await;
        
        let sync_state = sync_state.clone();
        let meetup_client = meetup_client.clone();
        let db_connection = db_connection.clone();
        let discord_api = discord_api.clone();
        
        tokio::spawn(async move {
            // Combine events from both sources
            let mut combined_collector = EventCollector::new();
            
            // Add Meetup events if available
            if let Some(meetup_collector) = sync_state.meetup_events.lock().await.as_ref() {
                for event in &meetup_collector.events {
                    combined_collector.add_event(event.clone());
                }
                println!("Added {} Meetup events to free spots", meetup_collector.events.len());
            }
            
            // Add SwissRPG events if available
            if let Some(swissrpg_collector) = sync_state.swissrpg_events.lock().await.as_ref() {
                for event in &swissrpg_collector.events {
                    combined_collector.add_event(event.clone());
                }
                println!("Added {} SwissRPG events to free spots", swissrpg_collector.events.len());
            }
            
            // Update Discord with free spots information
            if let Some(channel_id) = crate::discord::sync::ids::FREE_SPOTS_CHANNEL_ID {
                if let Err(err) = combined_collector
                    .update_channel(&discord_api, channel_id, static_file_prefix)
                    .await
                {
                    eprintln!("Error when posting open game spots:\n{:#?}", err);
                } else {
                    println!("Free spots Discord update completed successfully");
                }
            } else {
                eprintln!("No channel configured for posting open game spots");
            }
            
            // Assign roles based on combined events
            if let Err(err) = combined_collector
                .assign_roles(meetup_client, &db_connection, &discord_api)
                .await
            {
                eprintln!("Error in EventCollector::assign_roles:\n{:#?}", err);
            } else {
                println!("Role assignment completed successfully");
            }
        });
    }
}

#[deprecated(note = "Use the individual sync tasks instead: create_recurring_meetup_sync_task, create_recurring_swissrpg_sync_task, create_recurring_discord_sync_task, and create_recurring_free_spots_task")]
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
