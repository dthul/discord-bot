use askama::Error as AskamaError;
use chrono::format::ParseError as ChronoParseError;
use hyper::http::Error as HttpError;
use redis::RedisError;
use regex::Error as RegexError;
use reqwest::Error as ReqwestError;
use serenity::Error as SerenityError;
use simple_error::SimpleError;
use std::num::ParseIntError;
use tokio::timer::Error as TokioTimerError;
use url::ParseError as UrlParseError;

type RequestTokenError = oauth2::RequestTokenError<
    oauth2::reqwest::Error<reqwest::Error>,
    oauth2::StandardErrorResponse<oauth2::basic::BasicErrorResponseType>,
>;

#[derive(Debug)]
pub enum Error {
    APIError(super::api::Error),
    OAuthError(RequestTokenError),
    CommonError(crate::BoxedError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Encountered the following error:\n{:#?}", self)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::APIError(err) => Some(err),
            Error::OAuthError(_err) => None,
            Error::CommonError(err) => Some(err),
        }
    }
}

impl From<super::api::Error> for Error {
    fn from(err: super::api::Error) -> Self {
        Error::APIError(err)
    }
}

impl From<RequestTokenError> for Error {
    fn from(err: RequestTokenError) -> Self {
        Error::OAuthError(err)
    }
}

// impl<T: Into<common::BoxedError>> From<T> for Error {
//     fn from(err: T) -> Self {
//         Error::CommonError(err.into())
//     }
// }

impl From<SimpleError> for Error {
    fn from(err: SimpleError) -> Self {
        Error::CommonError(err.into())
    }
}

impl From<RedisError> for Error {
    fn from(err: RedisError) -> Self {
        Error::CommonError(err.into())
    }
}

impl From<ChronoParseError> for Error {
    fn from(err: ChronoParseError) -> Self {
        Error::CommonError(err.into())
    }
}

impl From<UrlParseError> for Error {
    fn from(err: UrlParseError) -> Self {
        Error::CommonError(err.into())
    }
}

impl From<SerenityError> for Error {
    fn from(err: SerenityError) -> Self {
        Error::CommonError(err.into())
    }
}

impl From<ReqwestError> for Error {
    fn from(err: ReqwestError) -> Self {
        Error::CommonError(err.into())
    }
}

impl From<HttpError> for Error {
    fn from(err: HttpError) -> Self {
        Error::CommonError(err.into())
    }
}

impl From<TokioTimerError> for Error {
    fn from(err: TokioTimerError) -> Self {
        Error::CommonError(err.into())
    }
}

impl From<AskamaError> for Error {
    fn from(err: AskamaError) -> Self {
        Error::CommonError(err.into())
    }
}

impl From<RegexError> for Error {
    fn from(err: RegexError) -> Self {
        Error::CommonError(err.into())
    }
}

impl From<ParseIntError> for Error {
    fn from(err: ParseIntError) -> Self {
        Error::CommonError(err.into())
    }
}
