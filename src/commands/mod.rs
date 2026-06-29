pub mod menu;
pub mod tg;
pub mod util;

pub mod admin;
pub mod assets;
pub mod balance;
pub mod bets;
pub mod betting;
pub mod buy;
pub mod checkin;
pub mod claim;
pub mod callbacks;
pub mod feedback;
pub mod predict;
pub mod predmarket;
pub mod settings;
pub mod markets;
pub mod referral;
pub mod rule;
pub mod sell;
pub mod selling;
pub mod send;
pub mod onlyreplyhere;
pub mod replyanywhere;
pub mod start;
pub mod timezone;

pub use admin::*;
pub use assets::*;
pub use balance::*;
pub use bets::*;
pub use buy::*;
pub use checkin::*;
pub use claim::*;
// `predict` and `feedback` each expose an `on_message` DM listener, so the two
// globs collide on that name in this namespace. It's harmless: nothing uses the
// bare `commands::on_message` — both listeners are reached via their module path
// (`crate::commands::{predict,feedback}::on_message`); the globs are only here to
// surface the `_COMMAND` statics for `create_framework!`.
#[allow(ambiguous_glob_reexports)]
pub use feedback::*;
#[allow(ambiguous_glob_reexports)]
pub use predict::*;
pub use settings::*;
pub use rule::*;
pub use markets::*;
pub use sell::*;
pub use send::*;
pub use onlyreplyhere::*;
pub use replyanywhere::*;
pub use start::*;
pub use timezone::*;
