use askama::Template;
use futures_util::lock::Mutex;
use hyper::{Body, Response};
use std::{borrow::Cow, future::Future, sync::Arc};
use warp::Filter;

pub enum HandlerResponse {
    Response(Response<Body>),
    Message {
        title: Cow<'static, str>,
        content: Option<Cow<'static, str>>,
        safe_content: Option<Cow<'static, str>>,
        img_url: Option<Cow<'static, str>>,
    },
}

impl HandlerResponse {
    pub fn from_template(template: impl Template) -> Result<Self, lib::meetup::Error> {
        template
            .render()
            .map_err(Into::into)
            .map(|html_body| HandlerResponse::Response(Response::new(html_body.into())))
    }
}

impl From<(&'static str, &'static str)> for HandlerResponse {
    fn from((title, content): (&'static str, &'static str)) -> Self {
        HandlerResponse::Message {
            title: Cow::Borrowed(title),
            content: Some(Cow::Borrowed(content)),
            safe_content: None,
            img_url: None,
        }
    }
}

impl From<(String, &'static str)> for HandlerResponse {
    fn from((title, content): (String, &'static str)) -> Self {
        HandlerResponse::Message {
            title: Cow::Owned(title),
            content: Some(Cow::Borrowed(content)),
            safe_content: None,
            img_url: None,
        }
    }
}

impl From<(&'static str, String)> for HandlerResponse {
    fn from((title, content): (&'static str, String)) -> Self {
        HandlerResponse::Message {
            title: Cow::Borrowed(title),
            content: Some(Cow::Owned(content)),
            safe_content: None,
            img_url: None,
        }
    }
}

impl From<(String, String)> for HandlerResponse {
    fn from((title, content): (String, String)) -> Self {
        HandlerResponse::Message {
            title: Cow::Owned(title),
            content: Some(Cow::Owned(content)),
            safe_content: None,
            img_url: None,
        }
    }
}

impl warp::Reply for HandlerResponse {
    fn into_response(self) -> warp::reply::Response {
        match self {
            HandlerResponse::Response(response) => response,
            HandlerResponse::Message {
                title,
                content,
                safe_content,
                img_url,
            } => {
                let rendering = super::MessageTemplate {
                    title: &title,
                    content: content.as_ref().map(Cow::as_ref),
                    safe_content: safe_content.as_ref().map(Cow::as_ref),
                    img_url: img_url.as_ref().map(Cow::as_ref),
                }
                .render();
                match rendering {
                    Ok(html_body) => Response::new(html_body.into()),
                    Err(err) => {
                        eprintln!("Error when trying to render MessageTemplate:\n{:#?}", err);
                        Response::new("Internal Server Error".into())
                    }
                }
            }
        }
    }
}

pub fn create_server(
    oauth2_consumer: Arc<lib::meetup::oauth2::OAuth2Consumer>,
    addr: std::net::SocketAddr,
    redis_client: redis::Client,
    async_meetup_client: Arc<Mutex<Option<Arc<lib::meetup::api::AsyncClient>>>>,
    bot_name: String,
) -> impl Future<Output = ()> + Send + 'static {
    let linking_routes = super::linking::create_routes(
        redis_client.clone(),
        oauth2_consumer.clone(),
        async_meetup_client.clone(),
        bot_name.clone(),
    );
    let schedule_session_routes = super::schedule_session::create_routes(
        redis_client.clone(),
        async_meetup_client.clone(),
        oauth2_consumer.clone(),
    );
    #[cfg(feature = "bottest")]
    let combined_routes = {
        let static_route = warp::path("static").and(warp::fs::dir("ui/src/web/html/static"));
        linking_routes.or(schedule_session_routes).or(static_route)
    };
    #[cfg(not(feature = "bottest"))]
    let combined_routes = linking_routes.or(schedule_session_routes);
    warp::serve(combined_routes).bind(addr)
}