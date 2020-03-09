mod sync_task;

use futures::future;
use futures_util::stream::StreamExt;
use redis::Commands;
use std::{env, pin::Pin, sync::Arc};

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
    let api_key = env::var("API_KEY").ok();
    if api_key.is_none() {
        eprintln!("No API key set. Will not listen to API requests.");
    }

    // Connect to the local Redis server
    let redis_url = if cfg!(feature = "bottest") {
        "redis://127.0.0.1/1"
    } else {
        "redis://127.0.0.1/0"
    };
    let redis_client = redis::Client::open(redis_url).expect("Could not create a Redis client");
    let mut redis_connection = redis_client
        .get_connection()
        .expect("Could not connect to Redis");

    // Create a Meetup API client (might not be possible if there is no access token yet)
    let meetup_access_token: Option<String> = redis_connection
        .get("meetup_access_token")
        .expect("Meetup access token could not be loaded from Redis");
    let async_meetup_client = match meetup_access_token {
        Some(meetup_access_token) => Arc::new(futures_util::lock::Mutex::new(Some(Arc::new(
            lib::meetup::api::AsyncClient::new(&meetup_access_token),
        )))),
        None => Arc::new(futures_util::lock::Mutex::new(None)),
    };

    // Create a Meetup OAuth2 consumer
    let meetup_oauth2_consumer = Arc::new(
        lib::meetup::oauth2::OAuth2Consumer::new(meetup_client_id, meetup_client_secret)
            .expect("Could not create a Meetup OAuth2 consumer"),
    );

    // Create a Stripe client
    let stripe_client = Arc::new(stripe::Client::new(&stripe_client_secret).with_headers(
        stripe::Headers {
            // stripe_version: Some(stripe::ApiVersion::V2019_03_14),
            ..Default::default()
        },
    ));

    // Create a task scheduler and schedule the refresh token task
    let task_scheduler = Arc::new(futures_util::lock::Mutex::new(
        white_rabbit::Scheduler::new(/*thread_count*/ 1),
    ));

    let (tx, rx) = futures_channel::mpsc::channel::<lib::BoxedFuture<()>>(1);
    let spawn_other_futures_future = rx.for_each(|fut| {
        let pinned_fut: Pin<Box<_>> = fut.into();
        lib::ASYNC_RUNTIME.spawn(pinned_fut);
        future::ready(())
    });

    let mut bot = ui::discord::bot::create_discord_client(
        &discord_token,
        redis_client.clone(),
        async_meetup_client.clone(),
        task_scheduler.clone(),
        tx,
        meetup_oauth2_consumer.clone(),
        stripe_client.clone(),
    )
    .expect("Could not create the Discord bot");
    let discord_api = lib::discord::CacheAndHttp {
        cache: bot.cache_and_http.cache.clone().into(),
        http: bot.cache_and_http.http.clone(),
    };

    // Start a server to handle Meetup OAuth2 logins
    let port = if cfg!(feature = "bottest") {
        3001
    } else {
        3000
    };
    let (abort_web_server_tx, abort_web_server_rx) = tokio::sync::oneshot::channel::<()>();
    let abort_web_server_signal = async {
        abort_web_server_rx.await.ok();
    };
    let web_server = ui::web::server::create_server(
        meetup_oauth2_consumer.clone(),
        ([127, 0, 0, 1], port).into(),
        redis_client.clone(),
        async_meetup_client.clone(),
        discord_api.clone(),
        bot.data
            .read()
            .get::<ui::discord::bot::BotNameKey>()
            .expect("Bot name was not set")
            .clone(),
        stripe_webhook_signing_secret,
        stripe_client,
        api_key,
        abort_web_server_signal,
    );

    // Organizer OAuth2 token refresh task
    let organizer_token_refresh_task = lib::tasks::token_refresh::organizer_token_refresh_task(
        (*meetup_oauth2_consumer).clone(),
        redis_client.clone(),
        async_meetup_client.clone(),
    );

    // Users OAuth2 token refresh task
    let users_token_refresh_task = lib::tasks::token_refresh::users_token_refresh_task(
        (*meetup_oauth2_consumer).clone(),
        redis_client.clone(),
    );

    // Schedule the end of game task
    let end_of_game_task = lib::tasks::end_of_game::create_end_of_game_task(
        redis_client.clone(),
        discord_api.clone(),
        bot.data
            .read()
            .get::<ui::discord::bot::BotIdKey>()
            .expect("Bot ID was not set")
            .clone(),
        /*recurring*/ true,
    );
    let next_end_of_game_task_time = {
        let mut task_time = white_rabbit::Utc::now().date().and_hms(18, 30, 0);
        // Check if it is later than 6:30pm
        // In that case, run the task tomorrow
        if white_rabbit::Utc::now() > task_time {
            task_time = task_time + white_rabbit::Duration::days(1);
        }
        task_time
    };
    let mut task_scheduler_guard =
        lib::ASYNC_RUNTIME.enter(|| futures::executor::block_on(task_scheduler.lock()));
    task_scheduler_guard.add_task_datetime(next_end_of_game_task_time, end_of_game_task);
    drop(task_scheduler_guard);

    let syncing_task = crate::sync_task::create_recurring_syncing_task(
        redis_client.clone(),
        async_meetup_client.clone(),
        discord_api,
        bot.data
            .read()
            .get::<ui::discord::bot::BotIdKey>()
            .expect("Bot ID was not set")
            .clone(),
        task_scheduler.clone(),
    );

    // Wrap the long-running tasks in abortable Futures
    let (organizer_token_refresh_task, abort_handle_organizer_token_refresh_task) =
        future::abortable(organizer_token_refresh_task);
    let (users_token_refresh_task, abort_handle_users_token_refresh_task) =
        future::abortable(users_token_refresh_task);
    let (spawn_other_futures_future, abort_handle_spawn_other_futures_future) =
        future::abortable(spawn_other_futures_future);
    let (syncing_task, abort_handle_syncing_task) = future::abortable(syncing_task);

    // Register the signal handlers
    let mut sigint_stream =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt());
    let mut sigterm_stream =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate());
    let signal_handling_task = async {
        // Wait until we receive a SIGINT (Ctrl+C) or a SIGTERM (systemd stop)
        tokio::select! {
            _ = sigint_stream.recv() => {},
            _ = sigterm_stream.recv() => {}
        }
    };

    // Spawn all tasks onto the async runtime
    lib::ASYNC_RUNTIME.enter(move || {
        lib::ASYNC_RUNTIME.spawn(spawn_other_futures_future);
        lib::ASYNC_RUNTIME.spawn(organizer_token_refresh_task);
        lib::ASYNC_RUNTIME.spawn(users_token_refresh_task);
        lib::ASYNC_RUNTIME.spawn(syncing_task);
        lib::ASYNC_RUNTIME.spawn(web_server);
    });

    // Finally, start the Discord bot
    if let Err(why) = bot.start() {
        println!("Client error: {:?}", why);
    }
}
