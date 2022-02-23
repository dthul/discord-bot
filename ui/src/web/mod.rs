pub mod api;
pub mod linking;
pub mod schedule_session;
pub mod server;
pub mod stripe_webhook_endpoint;

use std::borrow::Cow;

use askama::Template;
use axum::{
    body,
    http::status::StatusCode,
    response::{IntoResponse, Response},
};
use backtrace::Backtrace;
use hyper::header::InvalidHeaderValue;
use lib::BoxedError;
use redis::RedisError;

#[derive(Template)]
#[template(path = "message.html")]
struct MessageTemplate {
    title: Cow<'static, str>,
    content: Option<Cow<'static, str>>,
    safe_content: Option<Cow<'static, str>>,
    img_url: Option<Cow<'static, str>>,
}

impl From<(&'static str, &'static str)> for MessageTemplate {
    fn from((title, content): (&'static str, &'static str)) -> Self {
        MessageTemplate {
            title: Cow::from(title),
            content: Some(Cow::from(content)),
            safe_content: None,
            img_url: None,
        }
    }
}

impl From<(&'static str, String)> for MessageTemplate {
    fn from((title, content): (&'static str, String)) -> Self {
        MessageTemplate {
            title: Cow::from(title),
            content: Some(Cow::from(content)),
            safe_content: None,
            img_url: None,
        }
    }
}

// We can't implement IntoResponse for lib::meetup::Error in this crate so we create a new error type
#[derive(Debug)]
enum WebError {
    Lib(lib::meetup::Error),
    OAuthError(RequestTokenError),
    Other(BoxedError),
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let body = match self {
            WebError::Lib(err) => body::boxed(body::Full::from(format!(
                "Internal Server Error (Lib):\n{:#?}",
                err
            ))),
            WebError::OAuthError(err) => body::boxed(body::Full::from(format!(
                "Internal Server Error (OAuth):\n{:#?}",
                err
            ))),
            WebError::Other(err) => body::boxed(body::Full::from(format!(
                "Internal Server Error (Other):\n{:#?}",
                err
            ))),
        };
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(body)
            .unwrap()
    }
}

impl From<lib::meetup::Error> for WebError {
    fn from(err: lib::meetup::Error) -> Self {
        WebError::Lib(err)
    }
}

impl From<RedisError> for WebError {
    fn from(err: RedisError) -> Self {
        WebError::Other(err.into())
    }
}

impl From<InvalidHeaderValue> for WebError {
    fn from(err: InvalidHeaderValue) -> Self {
        WebError::Other(BoxedError {
            inner: Box::new(err),
            backtrace: Backtrace::new(),
        })
    }
}

type RequestTokenError = oauth2::RequestTokenError<
    oauth2::reqwest::AsyncHttpClientError,
    oauth2::StandardErrorResponse<oauth2::basic::BasicErrorResponseType>,
>;

impl From<RequestTokenError> for WebError {
    fn from(err: RequestTokenError) -> Self {
        WebError::OAuthError(err)
    }
}

impl From<lib::meetup::newapi::Error> for WebError {
    fn from(err: lib::meetup::newapi::Error) -> Self {
        WebError::Other(BoxedError {
            inner: Box::new(err),
            backtrace: Backtrace::new(),
        })
    }
}

impl From<sqlx::Error> for WebError {
    fn from(err: sqlx::Error) -> Self {
        WebError::Other(BoxedError {
            inner: Box::new(err),
            backtrace: Backtrace::new(),
        })
    }
}

impl From<oauth2::url::ParseError> for WebError {
    fn from(err: oauth2::url::ParseError) -> Self {
        WebError::Other(BoxedError {
            inner: Box::new(err),
            backtrace: Backtrace::new(),
        })
    }
}

impl From<askama::Error> for WebError {
    fn from(err: askama::Error) -> Self {
        WebError::Other(BoxedError {
            inner: Box::new(err),
            backtrace: Backtrace::new(),
        })
    }
}

impl From<axum::http::Error> for WebError {
    fn from(err: axum::http::Error) -> Self {
        WebError::Other(BoxedError {
            inner: Box::new(err),
            backtrace: Backtrace::new(),
        })
    }
}
