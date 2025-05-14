// use itertools::Itertools;
use crate::db;
use serenity::model::id::ChannelId;
// use unicode_segmentation::UnicodeSegmentation;

pub async fn say_in_event_series_channel(
    series_id: db::EventSeriesId,
    message: &str,
    db_connection: &sqlx::PgPool,
    discord_cache_http: &super::CacheAndHttp,
) -> Result<(), crate::BoxedError> {
    // Find the channel for this series id
    let channel_id = sqlx::query!(
        r#"SELECT discord_text_channel_id as "discord_text_channel_id!" FROM event_series WHERE id = $1"#,
        series_id.0
    )
    .map(|row| ChannelId::new(row.discord_text_channel_id as u64))
    .fetch_one(db_connection)
    .await?;
    channel_id
        .widen()
        .say(&discord_cache_http.http, message)
        .await?;
    Ok(())
}

pub async fn say_in_event_channel(
    event_id: db::EventId,
    message: &str,
    db_connection: &sqlx::PgPool,
    discord_cache_http: &super::CacheAndHttp,
) -> Result<(), crate::BoxedError> {
    // Find the channel for this event id
    let channel_id = sqlx::query!(
        r#"SELECT event_series.discord_text_channel_id as "discord_text_channel_id!"
        FROM event
        INNER JOIN event_series ON event.event_series_id = event_series.id
        WHERE event.id = $1"#,
        event_id.0
    )
    .map(|row| ChannelId::new(row.discord_text_channel_id as u64))
    .fetch_one(db_connection)
    .await?;
    channel_id
        .widen()
        .say(&discord_cache_http.http, message)
        .await?;
    Ok(())
}

pub async fn say_in_bot_alerts_channel(
    message: &str,
    discord_cache_http: &super::CacheAndHttp,
) -> Result<(), crate::BoxedError> {
    if let Some(channel_id) = super::sync::ids::BOT_ALERTS_CHANNEL_ID {
        channel_id
            .widen()
            .say(&discord_cache_http.http, message)
            .await
            .map(|_| ())
            .map_err(|err| err.into())
    } else {
        Err(simple_error::SimpleError::new(
            "Could not sent a bot alert message, since no bot alerts channel ID is set",
        )
        .into())
    }
}

// pub fn split_message(text: &'_ str) -> Vec<&'_ str> {
//     // Maximum number of Unicode code points in a message
//     const LIMIT: usize = serenity::constants::MESSAGE_CODE_LIMIT;
//     let mut parts = vec![];
//     let mut graphemes = UnicodeSegmentation::grapheme_indices(text, true)
//         .collect_vec()
//         .as_slice();
//     // Count the number of code points (scalar values) in each grapheme
//     let mut lengths = graphemes
//         .iter()
//         .map(|(_, grapheme)| grapheme.chars().count())
//         .collect_vec()
//         .as_slice();
//     while !graphemes.is_empty() {
//         // Find the next cut point
//         let mut current_length = 0;
//         let mut
//     }
//     for (&(offset, grapheme), &length) in graphemes.iter().zip(lengths.iter()) {}
//     let remaining = text.chars().collect_vec().as_slice();
//     let mut remaining = text;
//     while !remaining.is_empty() {}
//     parts
// }
