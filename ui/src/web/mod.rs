pub mod api;
pub mod auth;
pub mod linking;
pub mod schedule_session;
pub mod server;
pub mod stripe_webhook_endpoint;

use std::{backtrace::Backtrace, borrow::Cow};

use askama::Template;
use axum::{
    http::{status::StatusCode, uri::InvalidUri},
    response::{IntoResponse, Response},
};
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
pub enum WebError {
    // The variants below map to an internal server error
    Lib(lib::meetup::Error),
    OAuthError(RequestTokenError),
    Other(BoxedError),
    // The variants below map to an unauthorized HTTP status
    Unauthorized(Option<Cow<'static, str>>),
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        let body = match self {
            WebError::Lib(err) => format!("Internal Server Error (Lib):\n{:#?}", err).into(),
            WebError::OAuthError(err) => {
                format!("Internal Server Error (OAuth):\n{:#?}", err).into()
            }
            WebError::Other(err) => format!("Internal Server Error (Other):\n{:#?}", err).into(),
            WebError::Unauthorized(message) => {
                let template = MessageTemplate {
                    title: Cow::Borrowed("Unauthorized"),
                    content: message,
                    safe_content: None,
                    img_url: None,
                };
                let mut response = template.into_response();
                *response.status_mut() = StatusCode::UNAUTHORIZED;
                return response;
            }
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
            backtrace: Backtrace::force_capture(),
        })
    }
}

type RequestTokenError = oauth2::RequestTokenError<
    oauth2::HttpClientError<oauth2::reqwest::Error>,
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
            backtrace: Backtrace::force_capture(),
        })
    }
}

impl From<sqlx::Error> for WebError {
    fn from(err: sqlx::Error) -> Self {
        WebError::Other(BoxedError {
            inner: Box::new(err),
            backtrace: Backtrace::force_capture(),
        })
    }
}

impl From<oauth2::url::ParseError> for WebError {
    fn from(err: oauth2::url::ParseError) -> Self {
        WebError::Other(BoxedError {
            inner: Box::new(err),
            backtrace: Backtrace::force_capture(),
        })
    }
}

impl From<askama::Error> for WebError {
    fn from(err: askama::Error) -> Self {
        WebError::Other(BoxedError {
            inner: Box::new(err),
            backtrace: Backtrace::force_capture(),
        })
    }
}

impl From<axum::http::Error> for WebError {
    fn from(err: axum::http::Error) -> Self {
        WebError::Other(BoxedError {
            inner: Box::new(err),
            backtrace: Backtrace::force_capture(),
        })
    }
}

impl From<simple_error::SimpleError> for WebError {
    fn from(err: simple_error::SimpleError) -> Self {
        WebError::Other(BoxedError {
            inner: Box::new(err),
            backtrace: Backtrace::force_capture(),
        })
    }
}

impl From<InvalidUri> for WebError {
    fn from(err: InvalidUri) -> Self {
        WebError::Other(BoxedError {
            inner: Box::new(err),
            backtrace: Backtrace::force_capture(),
        })
    }
}
