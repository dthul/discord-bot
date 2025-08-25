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
