use super::CommandLevel;
use command_macro::command;
use serenity::{
    all::{CacheHttp, Mentionable},
    builder::{CreateEmbed, CreateMessage},
    model::id::UserId,
};
use std::fmt::Write;

// TODO: auto-generate the help command
#[command]
#[regex(r"help")]
#[help("help", "do I really need to explain this one?")]
fn help<'a>(
    context: &'a mut super::CommandContext,
    _: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    let bot_id = context.bot_id();
    let help_texts = compile_help_texts(bot_id);
    let is_bot_admin = context.is_admin().await.unwrap_or(false);
    let message_builder = CreateMessage::new()
        .content(lib::strings::HELP_MESSAGE_INTRO(bot_id))
        .embed(
            CreateEmbed::new()
                .colour(serenity::all::Colour::BLUE)
                .title(lib::strings::HELP_MESSAGE_PLAYER_EMBED_TITLE)
                .description(&help_texts.user),
        );
    context
        .msg
        .author
        .direct_message(context.ctx.http(), message_builder)
        .await
        .ok();
    let message_builder = CreateMessage::new().embed(
        CreateEmbed::new()
            .colour(serenity::all::Colour::DARK_GREEN)
            .title(lib::strings::HELP_MESSAGE_GM_EMBED_TITLE)
            .description(&help_texts.gm),
    );
    context
        .msg
        .author
        .direct_message(context.ctx.http(), message_builder)
        .await
        .ok();
    let message_builder = CreateMessage::new().embed(
        CreateEmbed::new()
            .colour(serenity::all::Colour::from_rgb(255, 23, 68))
            .title(lib::strings::HELP_MESSAGE_ADMIN_EMBED_TITLE)
            .description(&help_texts.admin),
    );
    if is_bot_admin {
        context
            .msg
            .author
            .direct_message(context.ctx.http(), message_builder)
            .await
            .ok();
    }
    Ok(())
}

struct HelpTexts {
    user: String,
    gm: String,
    admin: String,
}

// TODO: cache this
fn compile_help_texts(bot_id: UserId) -> HelpTexts {
    let mut user_help = String::new();
    let mut gm_help = String::new();
    let mut admin_help = String::new();
    for command in super::ALL_COMMANDS {
        let target = match command.level {
            CommandLevel::Everybody => &mut user_help,
            CommandLevel::HostAndAdminOnly => &mut gm_help,
            CommandLevel::AdminOnly => &mut admin_help,
        };
        for entry in command.help {
            writeln!(
                target,
                ":white_small_square: **{} {}** — {}",
                bot_id.mention(),
                entry.command,
                entry.explanation
            )
            .ok();
        }
    }
    HelpTexts {
        user: user_help,
        gm: gm_help,
        admin: admin_help,
    }
}
