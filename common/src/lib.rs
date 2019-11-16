pub mod error;
pub mod redis;
pub mod strings;

pub use error::BoxedError;
use lazy_static::lazy_static;

lazy_static! {
    pub static ref ASYNC_RUNTIME: tokio::runtime::Runtime =
        tokio::runtime::Runtime::new().expect("Could not create tokio runtime");
}

pub type BoxedFuture<T> = Box<dyn std::future::Future<Output = T> + Send>;
