use command_macro::command;
use serenity::builder::CreateMessage;

#[command]
#[regex(r"login")]
#[level(admin)]
#[help("login", "Log in to the web interface")]
fn login<'a>(
    context: &'a mut super::CommandContext,
    _: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    let user_id = context.msg.author.id;
    let url =
        crate::web::auth::generate_login_link(context.async_redis_connection().await?, user_id)
            .await?;
    let dm = context
        .msg
        .author
        .direct_message(
            &context.ctx.http,
            CreateMessage::new().content(lib::strings::LOGIN_LINK_MESSAGE(&url)),
        )
        .await;
    match dm {
        Ok(_) => {
            context.msg.react(&context.ctx.http, '\u{2705}').await.ok();
        }
        Err(why) => {
            eprintln!("Error sending login DM: {:?}", why);
            context
                .msg
                .reply(
                    &context.ctx.http,
                    "There was an error trying to send you a login link.\nDo you have direct \
                     messages disabled? In that case send me a private message with the text \
                     \"login\".",
                )
                .await
                .ok();
        }
    }
    Ok(())
}
