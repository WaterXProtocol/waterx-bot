pub mod menu;
pub mod tg;
pub mod util;

pub mod admin;
pub mod assets;
pub mod balance;
pub mod bets;
pub mod betting;
pub mod buy;
pub mod callbacks;
pub mod checkin;
pub mod feedback;
pub mod history;
pub mod markets;
pub mod onlyreplyhere;
pub mod predict;
pub mod predmarket;
pub mod referral;
pub mod replyanywhere;
pub mod rule;
pub mod sell;
pub mod selling;
pub mod send;
pub mod settings;
pub mod settle;
pub mod start;
pub mod timezone;

pub use admin::*;
pub use assets::*;
pub use balance::*;
pub use bets::*;
pub use buy::*;
pub use checkin::*;
pub use history::*;
// `predict` and `feedback` each expose an `on_message` DM listener, so the two
// globs collide on that name in this namespace. It's harmless: nothing uses the
// bare `commands::on_message` — both listeners are reached via their module path
// (`crate::commands::{predict,feedback}::on_message`); the globs are only here to
// surface the `_COMMAND` statics for `create_framework!`.
#[allow(ambiguous_glob_reexports)]
pub use feedback::*;
pub use markets::*;
pub use onlyreplyhere::*;
#[allow(ambiguous_glob_reexports)]
pub use predict::*;
pub use replyanywhere::*;
pub use rule::*;
pub use sell::*;
pub use send::*;
pub use settings::*;
pub use settle::*;
pub use start::*;
pub use timezone::*;
