pub mod discord;
pub mod error;
pub mod flow;
pub mod meetup;
pub mod redis;
pub mod strings;
pub mod stripe;
pub mod tasks;
pub mod urls;

pub use error::BoxedError;
use lazy_static::lazy_static;
use rand::Rng;

lazy_static! {
    pub static ref ASYNC_RUNTIME: tokio::runtime::Runtime = tokio::runtime::Builder::new()
        .enable_io()
        .enable_time()
        .threaded_scheduler()
        .build()
        .expect("Could not create tokio runtime");
}

pub type BoxedFuture<T> = Box<dyn std::future::Future<Output = T> + Send>;

pub fn new_random_id(num_bytes: u32) -> String {
    let random_bytes: Vec<u8> = (0..num_bytes)
        .map(|_| rand::thread_rng().gen::<u8>())
        .collect();
    base64::encode_config(&random_bytes, base64::URL_SAFE_NO_PAD)
}
