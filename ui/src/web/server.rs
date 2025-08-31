use std::{future::Future, sync::Arc};

use askama::Template;
use askama_axum::IntoResponse;
use axum::{
    extract::Extension,
    routing::{get, get_service},
    Router,
};
use futures_util::lock::Mutex;
use tower_http::services::ServeDir;

use super::{api, auth, linking, schedule_session, stripe_webhook_endpoint};

pub struct State {
    pub oauth2_consumer: Arc<lib::meetup::oauth2::OAuth2Consumer>,
    pub redis_client: redis::Client,
    pub pool: sqlx::PgPool,
    pub async_meetup_client: Arc<Mutex<Option<Arc<lib::meetup::newapi::AsyncClient>>>>,
    pub swissrpg_client: Arc<lib::swissrpg::client::SwissRPGClient>,
    pub discord_cache_http: lib::discord::CacheAndHttp,
    pub bot_name: String,
    pub stripe_webhook_secret: Option<String>,
    pub stripe_client: Arc<stripe::Client>,
    pub api_keys: Vec<String>,
}

#[derive(Template)]
#[template(path = "main.html")]
struct MainTemplate;

async fn main_handler() -> impl IntoResponse {
    MainTemplate
}

pub fn create_server(
    oauth2_consumer: Arc<lib::meetup::oauth2::OAuth2Consumer>,
    addr: std::net::SocketAddr,
    redis_client: redis::Client,
    pool: sqlx::PgPool,
    async_meetup_client: Arc<Mutex<Option<Arc<lib::meetup::newapi::AsyncClient>>>>,
    swissrpg_client: Arc<lib::swissrpg::client::SwissRPGClient>,
    discord_cache_http: lib::discord::CacheAndHttp,
    bot_name: String,
    stripe_webhook_secret: Option<String>,
    stripe_client: Arc<stripe::Client>,
    api_keys: Vec<String>,
    static_file_directory: String,
    shutdown_signal: impl Future<Output = ()> + Send + 'static,
) -> impl Future<Output = ()> + Send + 'static {
    let state = Arc::new(State {
        oauth2_consumer,
        redis_client,
        pool,
        async_meetup_client,
        swissrpg_client,
        discord_cache_http,
        bot_name,
        stripe_webhook_secret,
        stripe_client,
        api_keys,
    });
    let linking_routes = linking::create_routes();
    let schedule_session_routes = schedule_session::create_routes();
    let stripe_webhook_routes = stripe_webhook_endpoint::create_routes();
    let auth_routes = auth::create_routes();
    let api_routes = api::create_routes();
    let static_route: Router = Router::new().nest_service(
        "/static",
        get_service(ServeDir::new(static_file_directory).append_index_html_on_directories(false)),
    );
    let router = linking_routes
        .merge(schedule_session_routes)
        .merge(stripe_webhook_routes)
        .merge(auth_routes)
        .route(
            "/",
            get(main_handler).layer(axum::middleware::from_fn(auth::auth)),
        )
        .nest("/api", api_routes)
        .merge(static_route)
        .layer(Extension(state));
    async move {
        if let Err(err) = axum::Server::bind(&addr)
            .serve(router.into_make_service())
            .with_graceful_shutdown(shutdown_signal)
            .await
        {
            eprintln!("Web server exited with an error:\n{:#?}", err);
        }
    }
}
