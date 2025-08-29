use std::sync::Arc;

use futures_util::lock::Mutex;
use serenity::{all::Mentionable, builder::CreateMessage, model::id::RoleId};
use simple_error::SimpleError;

use super::free_spots::EventCollector;
use crate::db;

impl EventCollector {
    pub async fn assign_roles(
        &self,
        meetup_client: Arc<Mutex<Option<Arc<super::meetup::newapi::AsyncClient>>>>,
        db_connection: &sqlx::PgPool,
        discord_api: &crate::discord::CacheAndHttp,
    ) -> Result<(), crate::meetup::Error> {
        let meetup_client = {
            let guard = meetup_client.lock().await;
            match *guard {
                Some(ref meetup_client) => meetup_client.clone(),
                None => return Err(SimpleError::new("Meetup API unavailable").into()),
            }
            // The Mutex guard will be dropped here
        };
        println!("Role shortcode: Checking {} events", self.events.len());
        for event in &self.events {
            // Check whether this event uses the role shortcode
            let role_captures =
                crate::meetup::sync::ROLE_REGEX.captures_iter(event.description.as_deref().unwrap_or(""));
            let roles = {
                let mut roles = vec![];
                for captures in role_captures {
                    if let Some(role_id) = captures.name("role_id") {
                        match role_id.as_str().parse::<u64>() {
                            Ok(id) => roles.push(RoleId::new(id)),
                            _ => eprintln!(
                                "Event {} specifies invalid role id {}",
                                event.title,
                                role_id.as_str()
                            ),
                        }
                    }
                }
                roles
            };
            let title = &event.title;
            if roles.is_empty() {
                println!("Role shortcode: skipping {}", title);
                continue;
            }
            println!("Role shortcode: event {} has role(s)", title);
            // Skip SwissRPG events for now - only process Meetup events
            // Check if this looks like a Meetup ID (should be parseable as i64)
            let meetup_id = match event.id.parse::<i64>() {
                Ok(id) => id,
                Err(_) => {
                    println!("Role shortcode: skipping non-Meetup event {}", title);
                    continue;
                }
            };
            
            // Some events (games) might already have their RSVPs stored in the database.
            // For the others we query Meetup.
            let db_event_id = sqlx::query!(
                r#"SELECT event.id FROM meetup_event INNER JOIN event ON meetup_event.event_id = event.id WHERE meetup_event.meetup_id = $1"#,
                meetup_id.to_string()
            )
            .map(|row| db::EventId(row.id))
            .fetch_optional(db_connection)
            .await?;
            // Meetup user IDs
            let rsvps: Vec<db::Member> = if let Some(db_event_id) = db_event_id {
                // Get the RSVPs from the database
                println!("Role shortcode: event {} has RSVPs in the database", title);
                let hosts =
                    db::get_events_participants(&[db_event_id], true, db_connection).await?;
                let participants =
                    db::get_events_participants(&[db_event_id], false, db_connection).await?;
                hosts.into_iter().chain(participants.into_iter()).collect()
            } else {
                // Get the RSVPs from Meetup
                println!(
                    "Role shortcode: querying RSVPs for event {} from Meetup",
                    title
                );
                let meetup_participant_ids: Vec<_> =
                    match meetup_client.get_tickets_vec(meetup_id.to_string()).await {
                        Err(err) => {
                            eprintln!("Error in assign_roles::get_rsvps:\n{:#?}", err);
                            continue;
                        }
                        Ok(tickets) => tickets.iter().map(|ticket| ticket.user.id.0).collect(),
                    };
                // Look up the members corresponding to the Meetup IDs
                let members =
                    db::meetup_ids_to_members(&meetup_participant_ids, db_connection).await?;
                // Filter out null values
                members
                    .into_iter()
                    .filter_map(|(_, member)| member)
                    .map(Into::into)
                    .collect()
            };
            // Assign each role to each user
            for member in &rsvps {
                let discord_user_id = if let Some(discord_user_id) = member.discord_id {
                    discord_user_id
                } else {
                    continue;
                };
                let discord_member = match crate::discord::sync::ids::GUILD_ID
                    .member(discord_api, discord_user_id)
                    .await
                {
                    Err(err) => {
                        eprintln!("Could not find Discord discord_member:\n{:#?}", err);
                        continue;
                    }
                    Ok(discord_member) => discord_member,
                };
                for &role_id in &roles {
                    if !discord_member.roles.contains(&role_id) {
                        // Assign the role
                        if let Ok(_) = crate::tasks::subscription_roles::add_member_role(
                            discord_api,
                            discord_user_id,
                            role_id,
                            Some(
                                "Automatic role assignment due to being enrolled in an event with \
                                 role shortcode",
                            ),
                        )
                        .await
                        {
                            let role_text = role_id
                                .to_role_cached(&discord_api.cache)
                                .map(|role| format!("**{}**", role.name))
                                .unwrap_or_else(|| role_id.mention().to_string());
                            // Let the user know about the new role
                            if let Ok(user) = discord_user_id.to_user(discord_api).await {
                                user.direct_message(
                                    discord_api,
                                    CreateMessage::new()
                                        .content(crate::strings::NEW_ROLE_ASSIGNED_DM(&role_text)),
                                )
                                .await
                                .ok();
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
