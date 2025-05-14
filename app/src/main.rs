#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

use std::{
    env,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use futures::future;
use sqlx::{postgres::PgPoolOptions, Executor};

fn main() {
    let environment = env::var("BOT_ENV").expect("Found no BOT_ENV in environment");
    let is_test_environment = match environment.as_str() {
        "prod" => false,
        "test" => true,
        _ => panic!("BOT_ENV needs to be one of \"prod\" or \"test\""),
    };
    if cfg!(feature = "bottest") != is_test_environment {
        panic!(
            "The bot was compiled with bottest = {} but started with BOT_ENV = {}",
            cfg!(feature = "bottest"),
            environment
        );
    }
    let database_url = env::var("DATABASE_URL").expect("Found no DATABASE_URL in environment");
    let meetup_client_id =
        env::var("MEETUP_CLIENT_ID").expect("Found no MEETUP_CLIENT_ID in environment");
    let meetup_client_secret =
        env::var("MEETUP_CLIENT_SECRET").expect("Found no MEETUP_CLIENT_SECRET in environment");
    let discord_token = env::var("DISCORD_TOKEN").expect("Found no DISCORD_TOKEN in environment");
    let stripe_client_secret =
        env::var("STRIPE_CLIENT_SECRET").expect("Found no STRIPE_CLIENT_SECRET in environment");
    let stripe_webhook_signing_secret = env::var("STRIPE_WEBHOOK_SIGNING_SECRET").ok();
    if stripe_webhook_signing_secret.is_none() {
        eprintln!("No Stripe webhook signing secret set. Will not listen to Stripe webhooks.");
    }
    let api_keys: Vec<_> = env::var("API_KEYS")
        .as_deref()
        .unwrap_or("")
        .split(',')
        .filter(|key| key.len() >= 16)
        .map(|key| key.trim().to_string())
        .collect();
    if api_keys.len() == 0 {
        eprintln!("No API keys set. Will not listen to API requests.");
    }
    let static_file_directory = env::var("STATIC_FILE_DIRECTORY").ok();

    // Connect to the local Redis server
    let redis_url = if cfg!(feature = "bottest") {
        "redis://swissrpg-redis/1"
    } else {
        "redis://swissrpg-redis/0"
    };
    let redis_client = redis::Client::open(redis_url).expect("Could not create a Redis client");

    // Create a Meetup OAuth2 consumer
    let meetup_oauth2_consumer = Arc::new(
        lib::meetup::oauth2::OAuth2Consumer::new(meetup_client_id, meetup_client_secret)
            .expect("Could not create a Meetup OAuth2 consumer"),
    );

    // Create a Stripe client
    let stripe_client = Arc::new(stripe::Client::new(stripe_client_secret));

    // Create a tokio runtime
    let async_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()
        .expect("Could not create tokio runtime");

    // Connect to the local Postgres server
    let pool = loop {
        let pool_options = PgPoolOptions::new()
            .max_connections(5)
            .after_connect(|conn, _meta| {
                Box::pin(async move {
                    conn.execute("SET default_transaction_isolation TO 'serializable'")
                        .await?;
                    Ok(())
                })
            });
        match async_runtime.block_on(pool_options.connect(&database_url)) {
            Ok(pool) => break pool,
            Err(err) => {
                eprintln!("Could not connect to the Postgres database:\n{:#?}", err);
                println!("Retrying in 5 seconds");
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        }
    };

    // Create a Meetup API client (might not be possible if there is no access token yet)
    let meetup_access_token = async_runtime
        .block_on(async {
            sqlx::query_scalar!(r#"SELECT meetup_access_token FROM organizer_token"#)
                .fetch_optional(&pool)
                .await
        })
        .expect("Meetup access token could not be loaded from the database");
    let async_meetup_client = match meetup_access_token {
        Some(meetup_access_token) => Arc::new(futures_util::lock::Mutex::new(Some(Arc::new(
            lib::meetup::newapi::AsyncClient::new(&meetup_access_token),
        )))),
        None => Arc::new(futures_util::lock::Mutex::new(None)),
    };

    let bot_shutdown_signal = Arc::new(AtomicBool::new(false));
    let mut bot = async_runtime
        .block_on(ui::discord::bot::create_discord_client(
            &discord_token,
            redis_client.clone(),
            pool.clone(),
            async_meetup_client.clone(),
            meetup_oauth2_consumer.clone(),
            stripe_client.clone(),
            bot_shutdown_signal.clone(),
        ))
        .expect("Could not create the Discord bot");
    let discord_api = lib::discord::CacheAndHttp {
        cache: bot.cache.clone().into(),
        http: bot.http.clone(),
    };
    let bot_id = bot.data::<ui::discord::bot::UserData>().bot_id;
    let bot_name = bot.data::<ui::discord::bot::UserData>().bot_name.clone();

    // Start a server to handle Meetup OAuth2 logins
    let port = if cfg!(feature = "bottest") {
        3001
    } else {
        3000
    };
    let static_file_directory = static_file_directory.unwrap_or_else(|| {
        if cfg!(feature = "bottest") {
            "/usr/local/share/swissrpg-app-test/www".into()
        } else {
            "/usr/local/share/swissrpg-app/www".into()
        }
    });
    let (abort_web_server_tx, abort_web_server_rx) = tokio::sync::oneshot::channel::<()>();
    let abort_web_server_signal = async {
        abort_web_server_rx.await.ok();
    };
    let web_server = ui::web::server::create_server(
        meetup_oauth2_consumer.clone(),
        ([0, 0, 0, 0], port).into(),
        redis_client.clone(),
        pool.clone(),
        async_meetup_client.clone(),
        discord_api.clone(),
        bot_name,
        stripe_webhook_signing_secret,
        stripe_client.clone(),
        api_keys,
        static_file_directory,
        abort_web_server_signal,
    );

    // Organizer OAuth2 token refresh task
    let organizer_token_refresh_task = lib::tasks::token_refresh::organizer_token_refresh_task(
        (*meetup_oauth2_consumer).clone(),
        pool.clone(),
        async_meetup_client.clone(),
    );

    // // Users OAuth2 token refresh task
    // let users_token_refresh_task = lib::tasks::token_refresh::users_token_refresh_task(
    //     (*meetup_oauth2_consumer).clone(),
    //     pool.clone(),
    // );

    // Schedule the end of game task
    let end_of_game_task = lib::tasks::end_of_game::create_recurring_end_of_game_task(
        pool.clone(),
        discord_api.clone(),
        bot_id,
    );

    // User topic voice channel reset task
    let user_topic_voice_channel_reset_task =
        lib::tasks::user_topic_voice_channel::reset_user_topic_voice_channel_task(
            redis_client.clone(),
            discord_api.clone(),
        );

    let static_file_prefix = Box::leak(format!("{}/static/", lib::urls::BASE_URL).into_boxed_str());
    let syncing_task = lib::tasks::sync::create_recurring_syncing_task(
        pool.clone(),
        redis_client.clone(),
        async_meetup_client.clone(),
        discord_api.clone(),
        bot_id,
        static_file_prefix,
    );

    let stripe_subscription_refresh_task =
        lib::tasks::subscription_roles::stripe_subscriptions_refresh_task(
            discord_api.clone(),
            stripe_client.clone(),
        );

    // Wrap the long-running tasks in abortable Futures
    let (organizer_token_refresh_task, abort_handle_organizer_token_refresh_task) =
        future::abortable(organizer_token_refresh_task);
    // let (users_token_refresh_task, abort_handle_users_token_refresh_task) =
    //     future::abortable(users_token_refresh_task);
    let (end_of_game_task, abort_handle_end_of_game_task) = future::abortable(end_of_game_task);
    let (user_topic_voice_channel_reset_task, abort_handle_user_topic_voice_channel_reset_task) =
        future::abortable(user_topic_voice_channel_reset_task);
    let (syncing_task, abort_handle_syncing_task) = future::abortable(syncing_task);
    let (stripe_subscription_refresh_task, abort_handle_stripe_subscription_refresh_task) =
        future::abortable(stripe_subscription_refresh_task);

    // Create a synchronization barrier that keeps the main thread from exiting
    // until the signal handler tells it to
    let barrier = Arc::new(std::sync::Barrier::new(2));

    // Create the signal handling task
    let (signals_handle, signals_thread) = {
        let barrier = barrier.clone();
        let mut signals = signal_hook::iterator::Signals::new(&[
            signal_hook::consts::SIGINT,
            signal_hook::consts::SIGTERM,
        ])
        .expect("Could not register the signal handler");
        let handle = signals.handle();
        let thread = std::thread::spawn(move || {
            // Wait for SIGINT or SIGTERM to arrive
            for signal in &mut signals {
                match signal {
                    signal_hook::consts::SIGINT => println!("Received SIGINT. Shutting down."),
                    signal_hook::consts::SIGTERM => println!("Received SIGTERM. Shutting down."),
                    _ => unreachable!(),
                }
                // Finally, tell the main thread to exit
                barrier.wait();
                break;
            }
        });
        (handle, thread)
    };

    // Spawn all tasks onto the async runtime
    {
        let _runtime_guard = async_runtime.enter();
        tokio::spawn(async {
            let _ = organizer_token_refresh_task.await;
            println!("Organizer token refresh task shut down.");
        });
        // tokio::spawn(async {
        //     let _ = users_token_refresh_task.await;
        //     println!("User token refresh task shut down.");
        // });
        tokio::spawn(async {
            let _ = end_of_game_task.await;
            println!("End of game task shut down.");
        });
        tokio::spawn(async {
            let _ = user_topic_voice_channel_reset_task.await;
            println!("User topic voice channel reset task shut down.");
        });
        tokio::spawn(async {
            let _ = syncing_task.await;
            println!("Syncing task shut down.");
        });
        tokio::spawn(async {
            let _ = stripe_subscription_refresh_task.await;
            println!("Stripe subscription refresh task shut down.");
        });
        tokio::spawn(async {
            web_server.await;
            println!("Web server shut down.");
        });
        tokio::spawn(async move {
            if let Err(why) = bot.start().await {
                println!("Client error: {:#?}", why);
            }
            println!("Discord client shut down.");
        });
    }

    // Wait for a signal to exit the main thread
    barrier.wait();

    // We received the signal to shut down.
    // Stop the signal handler
    signals_handle.close();
    // Abort all long running futures
    bot_shutdown_signal.store(true, Ordering::Release);
    abort_handle_organizer_token_refresh_task.abort();
    // abort_handle_users_token_refresh_task.abort();
    abort_handle_end_of_game_task.abort();
    abort_handle_user_topic_voice_channel_reset_task.abort();
    abort_handle_syncing_task.abort();
    abort_handle_stripe_subscription_refresh_task.abort();
    abort_web_server_tx.send(()).ok();
    println!("About to shut down the tokio runtime.");
    // Give any currently running tasks a chance to finish
    async_runtime.shutdown_timeout(tokio::time::Duration::from_secs(20));
    println!("Tokio runtime shut down.");
    println!("Joining signal handling thread.");
    signals_thread
        .join()
        .expect("Could not join signal handling thread.");
    println!("Signal handling thread shut down.\nHyperion out.");
}
