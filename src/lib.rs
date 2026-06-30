// Money-handling bot: forbid `unsafe` crate-wide so the ledger/engine invariants
// can never be bypassed by raw memory access. We already use zero `unsafe`.
#![forbid(unsafe_code)]

pub mod bot;
pub mod commands;
pub mod core;
pub mod database;
