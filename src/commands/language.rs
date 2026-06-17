use crate::commands::util::*;
use crate::commands::{menu, tg};
use crate::i18n;
use telexide::prelude::*;

/// `/language` — re-open the language picker so a user can change their saved
/// locale. The `setlang:` callback persists the choice and opens the menu.
#[command(description = "change your language")]
pub async fn language(ctx: Context, message: Message) -> CommandResult {
    if paused_block(&ctx, &message).await? {
        return Ok(());
    }
    tg::send_with_buttons(
        &ctx,
        message.chat.get_id(),
        i18n::CHOOSE_LANGUAGE,
        &menu::lang_picker_rows(),
    )
    .await?;
    Ok(())
}
