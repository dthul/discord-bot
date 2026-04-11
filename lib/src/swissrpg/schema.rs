use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct User {
    #[serde(rename = "discordId")]
    pub discord_id: String,
    pub username: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Tag {
    pub code: String,
    pub value: String,
    #[serde(rename = "tagType")]
    pub tag_type: String,
}

pub(crate) const TAG_TYPE_LOCATION: &str = "location";
pub(crate) const LOCATION_CODE_ONLINE: &str = "online";

pub(crate) fn event_series_is_online(tags: &[Tag]) -> bool {
    tags.iter()
        .any(|tag| tag.tag_type == TAG_TYPE_LOCATION && tag.code == LOCATION_CODE_ONLINE)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Session {
    pub uuid: Uuid,
    pub number: i32,
    pub start: DateTime<Utc>,
    pub attendees: Vec<User>,
    #[serde(rename = "rsvpOpen")]
    pub rsvp_open: bool,
    #[serde(rename = "openSeats")]
    pub open_seats: i32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Event {
    pub uuid: Uuid,
    pub title: String,
    #[serde(rename = "publicUrl")]
    pub public_url: String,
    pub organisers: Vec<User>,
    pub description: Option<String>,
    #[serde(rename = "currentSession")]
    pub current_session: Option<Session>,
    #[serde(rename = "upcomingSessions")]
    pub upcoming_sessions: Vec<Session>,
    #[serde(rename = "legacyId")]
    pub legacy_id: Option<u64>,
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Location {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MigrateEventRequest {
    pub title: String,
    pub start: String,           // Format: "YYYY-MM-DD HH:MM"
    pub organisers: Vec<String>, // UUIDs as strings
    pub attendees: Vec<String>,  // UUIDs as strings
    #[serde(rename = "legacyId")]
    pub legacy_id: i64,
    pub description: Option<String>,
    pub end: Option<String>, // Format: "YYYY-MM-DD HH:MM"
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScheduleSessionRequest {
    pub start: String, // Format: "YYYY-MM-DD HH:MM"
    pub duration: i32, // Duration in minutes
    #[serde(rename = "includePlayers")]
    pub include_players: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiResponse<T> {
    pub data: T,
    pub success: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LocationsResponse {
    pub locations: Vec<Location>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_online_swissrpg_events_from_location_tag() {
        let tags = vec![Tag {
            code: LOCATION_CODE_ONLINE.to_string(),
            value: "Online".to_string(),
            tag_type: TAG_TYPE_LOCATION.to_string(),
        }];

        assert!(event_series_is_online(&tags));
    }

    #[test]
    fn ignores_non_location_online_like_tags() {
        let tags = vec![Tag {
            code: LOCATION_CODE_ONLINE.to_string(),
            value: "Online".to_string(),
            tag_type: "game_type".to_string(),
        }];

        assert!(!event_series_is_online(&tags));
    }
}
