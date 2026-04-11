pub mod client;
pub mod schema;
pub mod sync;

pub fn swissrpg_event_series_url(base_url: &str, event_series_id: &uuid::Uuid) -> String {
    format!(
        "{}/event/{}",
        base_url.trim_end_matches('/'),
        event_series_id
    )
}
