use command_macro::command;
use lib::{
    LinkingAction, LinkingMemberDiscord, LinkingMemberMeetup, LinkingResult, UnlinkingResult,
};
use serenity::{
    all::Mentionable,
    builder::{CreateEmbed, CreateMessage},
    model::id::UserId,
};
use std::borrow::Cow;

#[command]
#[regex(r"link[ -]?meetup")]
#[help("link meetup", "starts the process to link your Meetup and Discord profiles. If you haven't yet, you should really do that now.")]
fn link_meetup<'a>(
    context: &'a mut super::CommandContext,
    _: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    let pool = context.pool();
    // Check if there is already a meetup id linked to this user
    // and issue a warning
    let linked_meetup_id = sqlx::query!(
        r#"SELECT meetup_id FROM "member" WHERE discord_id = $1"#,
        context.msg.author.id.get() as i64
    )
    .map(|row| row.meetup_id.map(|id| id as u64))
    .fetch_optional(&pool)
    .await?
    .flatten();
    if let Some(linked_meetup_id) = linked_meetup_id {
        let bot_id = context.bot_id();
        let message_builder =
            CreateMessage::new().content(lib::strings::DISCORD_ALREADY_LINKED_MESSAGE(
                &format!("https://www.meetup.com/members/{}/", linked_meetup_id),
                bot_id,
            ));
        context
            .msg
            .author
            .direct_message(&context.ctx.http, message_builder)
            .await
            .ok();
        return Ok(());
    };
    let user_id = context.msg.author.id;
    let url = lib::meetup::oauth2::generate_meetup_linking_link(
        context.async_redis_connection().await?,
        user_id,
    )
    .await?;
    let message_builder = CreateMessage::new().content(lib::strings::MEETUP_LINKING_MESSAGE(&url));
    let dm = context
        .msg
        .author
        .direct_message(&context.ctx.http, message_builder)
        .await;
    match dm {
        Ok(_) => {
            context.msg.react(&context.ctx.http, '\u{2705}').await.ok();
        }
        Err(why) => {
            eprintln!("Error sending Meetup linking DM: {:?}", why);
            context.msg.reply(
                &context.ctx.http,
                "There was an error trying to send you instructions.\nDo you have direct messages \
                 disabled? In that case send me a private message with the text \"link meetup\".",
            ).await.ok();
        }
    }
    Ok(())
}

#[command]
#[regex(r"unlink[ -]?meetup")]
#[help("unlink meetup", "unlinks your Meetup and Discord profiles.")]
fn unlink_meetup<'a>(
    context: &'a mut super::CommandContext,
    _: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    let user_id = context.msg.author.id;
    unlink_meetup_impl(context, /*is_bot_admin_command*/ false, user_id).await
}

#[command]
#[regex(
    r"link[ -]?meetup\s+{mention_pattern}\s+(?P<meetupid>[0-9]+)",
    mention_pattern
)]
#[level(admin)]
#[help(
    "link meetup `@some-user` `meetup-ID`",
    "link another user's Meetup and Discord profile."
)]
fn link_meetup_bot_admin<'a>(
    context: &'a mut super::CommandContext,
    captures: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    let discord_id = captures.name("mention_id").unwrap().as_str();
    let meetup_id = captures.name("meetupid").unwrap().as_str();
    // Try to convert the specified ID to an integer
    let (discord_id, meetup_id) = match (discord_id.parse::<u64>(), meetup_id.parse::<u64>()) {
        (Ok(id1), Ok(id2)) => (UserId::from(id1), id2),
        _ => {
            let _ = context.msg.channel_id.say(
                &context.ctx.http,
                "Seems like the specified Discord or Meetup ID is invalid",
            );
            return Ok(());
        }
    };
    let pool = context.pool();
    let mut tx = pool.begin().await?;
    let linking_result = lib::link_discord_meetup(discord_id, meetup_id, &mut tx).await?;
    tx.commit().await?;
    match linking_result {
        LinkingResult::Success {
            action: LinkingAction::AlreadyLinked,
            ..
        } => {
            context
                .msg
                .channel_id
                .say(
                    &context.ctx.http,
                    format!(
                        "All good, this Meetup account was already linked to {}",
                        discord_id.mention()
                    ),
                )
                .await
                .ok();
        }
        LinkingResult::Success {
            action: LinkingAction::Linked | LinkingAction::NewMember | LinkingAction::MergedMember,
            ..
        } => {
            let message_builder = CreateMessage::new().embed(
                CreateEmbed::new().title("Linked Meetup account").description(format!(
                    "Successfully linked {} to this Meetup account: https://www.meetup.com/members/{}/",
                    discord_id.mention(), meetup_id
                ))
            );
            let _ = context
                .msg
                .channel_id
                .send_message(&context.ctx.http, message_builder);
        }
        LinkingResult::Conflict {
            member_with_meetup:
                LinkingMemberMeetup {
                    meetup_id: meetup_id1,
                    discord_id: discord_id1,
                    ..
                },
            member_with_discord:
                LinkingMemberDiscord {
                    meetup_id: meetup_id2,
                    discord_id: discord_id2,
                    ..
                },
        } => {
            let message = format!("The specified Meetup and Discord IDs are attached to different user profiles \
            but those profiles can't be merged because at least one of the profiles already has a conflicting \
            linking:\n\n\
            **User profile 1**\n\
            Meetup ID: {meetup_id1}\n\
            Discord ID: {discord_id1:?}\n\n\
            **User profile 2**\n\
            Meetup ID: {meetup_id2:?}\n\
            Discord ID: {discord_id2:?}");
            // TODO: answer in DM?
            context
                .msg
                .channel_id
                .say(&context.ctx.http, message)
                .await
                .ok();
        }
    };
    Ok(())
}

#[command]
#[regex(r"unlink[ -]?meetup\s+{mention_pattern}", mention_pattern)]
#[level(admin)]
#[help(
    "unlink meetup `@some-user`",
    "unlink another user's Meetup and Discord profile."
)]
fn unlink_meetup_bot_admin<'a>(
    context: &'a mut super::CommandContext,
    captures: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    let discord_id = captures.name("mention_id").unwrap().as_str();
    // Try to convert the specified ID to an integer
    let discord_id = match discord_id.parse::<u64>() {
        Ok(id) => UserId::from(id),
        _ => {
            let _ = context.msg.channel_id.say(
                &context.ctx.http,
                "Seems like the specified Discord ID is invalid",
            );
            return Ok(());
        }
    };
    unlink_meetup_impl(context, /*is_bot_admin_command*/ true, discord_id).await
}

async fn unlink_meetup_impl(
    context: &mut super::CommandContext,
    is_bot_admin_command: bool,
    user_id: UserId,
) -> Result<(), lib::meetup::Error> {
    let pool = context.pool();
    let mut tx = pool.begin().await?;
    let result = lib::unlink_meetup(user_id, &mut tx).await?;
    tx.commit().await?;
    match result {
        UnlinkingResult::Success => {
            let message = if is_bot_admin_command {
                format!("Unlinked {}'s Meetup account", user_id.mention())
            } else {
                lib::strings::MEETUP_UNLINK_SUCCESS(context.bot_id())
            };
            context
                .msg
                .channel_id
                .say(&context.ctx.http, message)
                .await
                .ok();
        }
        UnlinkingResult::NotLinked => {
            let message = if is_bot_admin_command {
                Cow::Owned(format!(
                    "There was seemingly no meetup account linked to {}",
                    user_id.mention()
                ))
            } else {
                Cow::Borrowed(lib::strings::MEETUP_UNLINK_NOT_LINKED)
            };
            context
                .msg
                .channel_id
                .say(&context.ctx.http, message)
                .await
                .ok();
        }
    }
    Ok(())
}
