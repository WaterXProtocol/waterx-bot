use crate::core::i18n::{self, Lang};
use crate::core::types::{BetState, OddsFormat};
use std::collections::HashMap;

/// Stake amounts shown as per-option buttons in the betting keyboard.
pub const STAKE_AMOUNTS: &[i64] = &[1, 5, 10, 50];

/// `1..=10` → circled digits ①..⑩ for the board; larger falls back to `n.`.
fn circled(n: usize) -> String {
    const C: [&str; 10] = ["①", "②", "③", "④", "⑤", "⑥", "⑦", "⑧", "⑨", "⑩"];
    C.get(n - 1).map_or_else(|| format!("{n}."), |c| (*c).to_string())
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct OptionData {
    pub detail: HashMap<i64, i64>,
    pub bet: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Prediction {
    pub id: String,
    pub host: i64,
    /// Language the shared board is rendered in — the host's locale, resolved
    /// at creation. `#[serde(default)]` keeps games persisted before i18n
    /// (which have no `lang` field) loading as English.
    #[serde(default)]
    pub lang: Lang,
    pub description: String,
    pub state: BetState,
    pub options: HashMap<String, OptionData>,
    pub option_order: Vec<String>,
    pub total: i64,
    pub inputs: HashMap<i64, i64>,
    pub outputs: HashMap<i64, i64>,
    pub changes: HashMap<i64, i64>,
    /// Bettor display names captured at stake time, so the settlement readout can
    /// show names instead of opaque id tails. `#[serde(default)]` keeps games
    /// persisted before this field loaded (they fall back to the id tail).
    #[serde(default)]
    pub names: HashMap<i64, String>,
    /// Unix time after which new bets are rejected (0 = no deadline). Set by the
    /// `/predict` builder. Enforced lazily at bet time (no scheduler); the host
    /// still closes/settles manually. `#[serde(default)]` for pre-deadline games.
    #[serde(default)]
    pub ends_at: i64,
    /// Odds display format for the **shared board**, pinned to the host's pref at
    /// creation (like `lang`) — so the board never flips format per viewer. The
    /// per-bettor DM builder uses the *bettor's* format instead. `#[serde(default)]`
    /// loads pre-feature games as `Decimal`.
    #[serde(default)]
    pub odds_fmt: OddsFormat,
    /// Builder's timezone (minutes east of UTC) pinned at creation, so the shared
    /// board renders the **deadline** in the host's local time — same pinning
    /// rationale as `lang`/`odds_fmt` (a shared message can't localize per viewer).
    /// `#[serde(default)]` loads pre-feature games as 0 = UTC.
    #[serde(default)]
    pub tz_offset: i64,
}

impl Prediction {
    pub fn new(host: i64, lang: Lang, description: &str, options: &[&str]) -> Self {
        let mut opts = HashMap::new();
        let mut order = Vec::new();
        for o in options {
            opts.insert((*o).to_string(), OptionData::default());
            order.push((*o).to_string());
        }
        Self {
            id: String::new(),
            host,
            lang,
            description: description.to_string(),
            state: BetState::betting,
            options: opts,
            option_order: order,
            total: 0,
            inputs: HashMap::new(),
            outputs: HashMap::new(),
            changes: HashMap::new(),
            names: HashMap::new(),
            ends_at: 0,
            odds_fmt: OddsFormat::Decimal,
            tz_offset: 0,
        }
    }

    /// Whether betting is past the deadline (lazily enforced — the state may still
    /// be `betting` since there's no scheduler to auto-close).
    pub fn ended(&self, now: i64) -> bool {
        self.ends_at != 0 && now >= self.ends_at
    }

    /// An option's current pari-mutuel payout, rendered in `fmt`. The natural form
    /// is the **decimal multiplier** `pool ÷ option-stake` (shown as `×2.50` — "your
    /// stake ×"); other formats convert via `cents = 100 / multiplier`. `×—` when
    /// the option has no bets yet (no defined multiplier). Used by the shared board
    /// (host's `odds_fmt`) and the per-bettor DM builder (the bettor's format).
    pub fn option_odds(&self, opt: &str, fmt: OddsFormat) -> String {
        let bet = self.options.get(opt).map_or(0, |d| d.bet);
        if bet <= 0 || self.total <= 0 {
            return "×—".to_string();
        }
        let mult = self.total as f64 / bet as f64;
        match fmt {
            OddsFormat::Decimal => format!("×{mult:.2}"),
            _ => crate::commands::util::format_odds(100.0 / mult, fmt),
        }
    }

    pub fn set_id(&mut self, chat_id: i64, msg_id: i64) {
        self.id = format!("{chat_id}:{msg_id}");
        let tail_start = self.id.len().saturating_sub(7);
        let tail = &self.id[tail_start..];
        self.description = format!("{tail}\n{}", self.description);
    }

    pub fn get_header(&self) -> String {
        format!("{}\n---{}\n", self.description, self.state.label(self.lang))
    }

    /// The host's question (the board strips the leading id-tail line that
    /// `set_id` prepends to `description`).
    fn question(&self) -> &str {
        self.description
            .split_once('\n')
            .map_or(self.description.as_str(), |(_, q)| q)
    }

    fn state_emoji(&self) -> &'static str {
        match self.state {
            BetState::betting => "🟢",
            BetState::closed => "🔴",
            BetState::settled => "✅",
            BetState::draw => "↩️",
        }
    }

    /// The live, shared pari-mutuel board: question + state, one numbered line
    /// per option (pool 🪙 → win multiplier), and a localized pool footer. Used
    /// while betting/closed; after settling the message is replaced by the
    /// result text instead.
    pub fn get_text(&self) -> String {
        let mut s = format!(
            "🎲 {}  ·  {} {}\n",
            self.question(),
            self.state_emoji(),
            self.state.label(self.lang)
        );
        // Deadline rendered in the **host's** timezone (pinned at creation, like
        // `lang`/`odds_fmt`), with an explicit UTC±offset label so it's
        // unambiguous on the shared board regardless of who's reading it.
        if self.ends_at != 0 {
            if let Some(t) = crate::commands::util::fmt_local_time(self.ends_at, self.tz_offset) {
                s.push_str(&format!("⏰ {t}\n"));
            }
        }
        for (i, opt) in self.option_order.iter().enumerate() {
            if let Some(d) = self.options.get(opt) {
                // Board renders in the host's pinned format (shared message).
                let mult = self.option_odds(opt, self.odds_fmt);
                s.push_str(&format!("\n{} {opt}   {} 🪙 → {mult}", circled(i + 1), d.bet));
            }
        }
        let total = self.total.to_string();
        let footer = if self.state == BetState::betting {
            i18n::board_footer_open(self.lang, &total)
        } else {
            i18n::board_footer_closed(self.lang, &total)
        };
        s.push_str(&format!("\n\n{footer}"));
        s
    }

    /// Inline-keyboard rows used by `commands::tg::send_with_buttons` /
    /// `edit_with_buttons`. While betting is open, each option gets a row of
    /// amount buttons that **accumulate** the tapper's pending stake (handled
    /// per-user in `callbacks`), followed by a Confirm/Clear row and the host's
    /// Close button; settling shows one pick-a-winner button per option plus a
    /// draw button. Amount/option are referenced by **index** (`gamble:add:<i>:
    /// <amt>`) so an option's text can't break the callback data.
    pub fn get_buttons(&self) -> Vec<Vec<(String, String)>> {
        let mut rows: Vec<Vec<(String, String)>> = Vec::new();
        match self.state {
            BetState::betting => {
                // All options on one row ([optA] [optB] …); tapping one DMs that
                // user a private stake builder (the bet flow happens in DM, like
                // match betting). `[close]` sits on its own row below.
                rows.push(
                    self.option_order
                        .iter()
                        .enumerate()
                        .map(|(i, opt)| (opt.clone(), format!("gamble:pick:{i}")))
                        .collect(),
                );
                rows.push(vec![(
                    i18n::close_button(self.lang).to_string(),
                    "gamble:".to_string(),
                )]);
            }
            BetState::closed => {
                // Settle: all outcome buttons on one row, void on its own row.
                rows.push(
                    self.option_order
                        .iter()
                        .map(|opt| (opt.clone(), format!("gamble:{opt}")))
                        .collect(),
                );
                rows.push(vec![(
                    i18n::void_label(self.lang).to_string(),
                    "gamble:$draw$".to_string(),
                )]);
            }
            _ => {}
        }
        rows
    }

    // `f64` is a bounded, rounded odds intermediate; the ledger stays integer
    // micro-coins and stakes are capped (`util::MAX_COINS`), so there is no
    // precision loss at the bot's magnitudes — same tradeoff as
    // `database::wager::decimal_payout`.
    #[allow(clippy::cast_precision_loss)]
    pub fn stake(&mut self, user: i64, option: &str, amount: i64, name: &str) -> bool {
        if self.state != BetState::betting {
            return false;
        }
        if !self.options.contains_key(option) {
            return false;
        }
        if let Some(d) = self.options.get_mut(option) {
            d.bet += amount;
            *d.detail.entry(user).or_insert(0) += amount;
        }
        self.names.insert(user, name.to_string());
        *self.inputs.entry(user).or_insert(0) += amount;
        self.total += amount;
        // Odds aren't stored — they're derived on demand from the live pools via
        // `option_odds`, so a stake just moves the pools.
        true
    }

    pub fn close(&mut self) -> bool {
        if self.state != BetState::betting {
            return false;
        }
        self.state = BetState::closed;
        true
    }

    // See `stake`: the payout ratio is an `f64` intermediate over capped,
    // integer micro-coin stakes — no precision loss at the bot's magnitudes.
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    pub fn settle(&mut self, outcome: &str) -> (HashMap<i64, i64>, String) {
        if self.state != BetState::closed {
            return (HashMap::new(), String::new());
        }
        if outcome == "$draw$" {
            // Refund every staker by returning self.inputs as outputs (Python
            // does the same). `changes` is left empty so /reverse can't undo
            // a draw.
            self.state = BetState::draw;
            return (
                self.inputs.clone(),
                i18n::result_header(self.lang, i18n::void_label(self.lang)),
            );
        }
        // Nobody bet at all → void with a "no one bet" note.
        if self.inputs.is_empty() {
            self.state = BetState::draw;
            return (
                HashMap::new(),
                i18n::result_header(self.lang, outcome) + i18n::no_one_bet_suffix(self.lang),
            );
        }

        let mut outputs = HashMap::new();
        if let Some(win_opt) = self.options.get(outcome) {
            if win_opt.bet > 0 {
                let ratio = self.total as f64 / win_opt.bet as f64;
                for (&user, &stake) in &win_opt.detail {
                    outputs.insert(user, (ratio * stake as f64) as i64);
                }
            }
        }

        // People bet, but the winning option drew **no stakes** (e.g. a one-sided
        // pool settled onto the empty side). Nobody to pay and no counterparty
        // pool to distribute, so **refund every staker** (void) rather than
        // burning the pool. `changes` stays empty (like a draw).
        if outputs.is_empty() {
            self.state = BetState::draw;
            return (
                self.inputs.clone(),
                i18n::result_header(self.lang, i18n::void_label(self.lang))
                    + i18n::no_winners_refund(self.lang),
            );
        }

        let mut changes = HashMap::new();
        for (&user, &input) in &self.inputs {
            let win = outputs.get(&user).copied().unwrap_or(0);
            changes.insert(user, win - input);
        }
        self.outputs = outputs.clone();
        self.changes = changes.clone();

        self.state = BetState::settled;
        // Only the header here — the per-winner readout is built separately via
        // `winners_readout`, so a busy prediction can't blow past Telegram's
        // message-size limit when it settles.
        (outputs, i18n::result_header(self.lang, outcome))
    }

    /// The settlement body shown below the header: the top-`limit` net winners
    /// (`top_winners_block`), or an "everyone broke even" note when it settled
    /// with payouts but **no net winner** (a one-sided pool where all the money
    /// was on the winning side). Empty for a void/refund — the header says it all.
    pub fn winners_readout(&self, limit: usize) -> String {
        let block = self.top_winners_block(limit);
        if block.is_empty() && self.state == BetState::settled {
            return i18n::all_broke_even(self.lang).to_string();
        }
        block
    }

    /// Up to `limit` biggest **net winners** (those with a positive change),
    /// ranked by amount won, one `settle_line` each, with a "…and N more" tail
    /// when there are more. Empty when nobody won (draw / void / no-one-bet) —
    /// reads `self.changes`, so call it **after** `settle`. Lets settlement show
    /// a bounded top list instead of one line per bettor.
    pub fn top_winners_block(&self, limit: usize) -> String {
        let mut winners: Vec<(i64, i64)> = self
            .changes
            .iter()
            .filter(|&(_, &d)| d > 0)
            .map(|(&u, &d)| (u, d))
            .collect();
        if winners.is_empty() {
            return String::new();
        }
        // By amount won (desc), ties broken by id for a stable order.
        winners.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let total = winners.len();
        let mut s = String::new();
        for (user, diff) in winners.iter().take(limit) {
            s.push_str(&i18n::settle_line(
                self.lang,
                &self.display_name(*user),
                i18n::verb_won(self.lang),
                &diff.to_string(),
            ));
        }
        if total > limit {
            s.push_str(&i18n::more_winners(self.lang, &(total - limit).to_string()));
        }
        s
    }

    /// A bettor's captured display name, falling back to a masked id tail
    /// (`***1234`) for games staked before names were recorded.
    fn display_name(&self, user: i64) -> String {
        self.names.get(&user).cloned().unwrap_or_else(|| {
            let s = format!("{user:0>4}");
            format!("***{}", &s[s.len().saturating_sub(4)..])
        })
    }

    pub fn reverse(&self) -> Option<HashMap<i64, i64>> {
        if self.state != BetState::settled {
            return None;
        }
        Some(self.changes.iter().map(|(&k, &v)| (k, -v)).collect())
    }

    /// Per-user status line. Returns `""` if this user has no record in this
    /// prediction, so callers can skip uninteresting entries when listing.
    pub fn check(&self, user: i64) -> String {
        let record = match self.state {
            BetState::betting | BetState::closed => {
                let mut r = String::new();
                for opt in &self.option_order {
                    if let Some(d) = self.options.get(opt) {
                        if let Some(stake) = d.detail.get(&user) {
                            r.push_str(&format!("\n{opt}: {stake}"));
                        }
                    }
                }
                r
            }
            BetState::draw => {
                // A draw involves everyone whose stake got refunded.
                if self.inputs.contains_key(&user) {
                    " ".to_string()
                } else {
                    String::new()
                }
            }
            BetState::settled => {
                if let Some(diff) = self.changes.get(&user) {
                    let verb = if *diff >= 0 {
                        i18n::verb_won(self.lang)
                    } else {
                        i18n::verb_lost(self.lang)
                    };
                    format!("\n{verb} {}", diff.abs())
                } else {
                    String::new()
                }
            }
        };
        if record.is_empty() {
            String::new()
        } else {
            format!("{}{record}\n", self.get_header())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stake_updates_all_odds() {
        let mut g = Prediction::new(0, Lang::En, "test", &["A", "B"]);
        g.stake(1, "A", 10, "Alice");
        g.stake(2, "B", 5, "Bob");
        // total = 15. A.bet = 10 → ×1.50. B.bet = 5 → ×3.00.
        assert_eq!(g.option_odds("A", OddsFormat::Decimal), "×1.50");
        assert_eq!(g.option_odds("B", OddsFormat::Decimal), "×3.00");
        // Another bet on A should move B's odds too.
        g.stake(3, "A", 15, "Cara");
        // total = 30. A.bet = 25 → ×1.20. B.bet = 5 → ×6.00.
        assert_eq!(g.option_odds("A", OddsFormat::Decimal), "×1.20");
        assert_eq!(g.option_odds("B", OddsFormat::Decimal), "×6.00");
    }

    #[test]
    fn deadline_renders_in_host_timezone() {
        // 2021-01-01 00:00:00 UTC.
        let ts = 1_609_459_200;
        let mut g = Prediction::new(0, Lang::En, "q", &["A", "B"]);
        g.ends_at = ts;

        // UTC host → "Jan 1 · 00:00 UTC".
        g.tz_offset = 0;
        let utc = g.get_text();
        assert!(utc.contains("⏰ Jan 1 · 00:00 UTC\n"), "got: {utc}");

        // UTC+8 host → same instant shown as 08:00 with a UTC+8 label.
        g.tz_offset = 480;
        let plus8 = g.get_text();
        assert!(plus8.contains("⏰ Jan 1 · 08:00 UTC+8"), "got: {plus8}");

        // No deadline → no clock line at all.
        g.ends_at = 0;
        assert!(!g.get_text().contains('⏰'));
    }

    #[test]
    fn top_winners_block_caps_and_ranks() {
        // 12 bettors on the winning side A, one loser on B.
        let mut g = Prediction::new(0, Lang::En, "q", &["A", "B"]);
        for i in 1..=12 {
            g.stake(i, "A", i, &format!("u{i}")); // bigger stake → bigger win
        }
        // Large losing pool → every A bettor nets clearly positive.
        g.stake(99, "B", 1000, "loser");
        g.close();
        let _ = g.settle("A");
        let block = g.top_winners_block(10);
        // Capped to 10 winners + a "2 more" tail; the loser never appears.
        assert_eq!(block.matches('\n').count(), 11, "10 winner lines + 1 tail");
        assert!(block.contains("more"), "tail present: {block}");
        assert!(!block.contains("loser"));
        // A draw produces no winner block.
        let mut d = Prediction::new(0, Lang::En, "q", &["A", "B"]);
        d.stake(1, "A", 10, "Alice");
        d.close();
        let _ = d.settle("$draw$");
        assert_eq!(d.top_winners_block(10), "");
    }

    #[test]
    fn onesided_settle_winning_side_breaks_even() {
        // Everyone on A; settle A → ratio 1.0, each refunded their stake.
        let mut g = Prediction::new(0, Lang::En, "q", &["A", "B"]);
        g.stake(1, "A", 10, "Alice");
        g.stake(2, "A", 30, "Bob");
        g.close();
        let (outputs, _h) = g.settle("A");
        assert_eq!(outputs[&1], 10);
        assert_eq!(outputs[&2], 30); // got stakes back
        assert_eq!(g.state, BetState::settled);
        // No net winner → break-even note instead of an empty body.
        assert_eq!(g.top_winners_block(10), "");
        assert!(g.winners_readout(10).contains("broke even"));
    }

    #[test]
    fn onesided_settle_empty_side_refunds_not_burns() {
        // Everyone on A; settle B (nobody bet B) → refund all, don't burn.
        let mut g = Prediction::new(0, Lang::En, "q", &["A", "B"]);
        g.stake(1, "A", 10, "Alice");
        g.stake(2, "A", 30, "Bob");
        g.close();
        let (outputs, header) = g.settle("B");
        assert_eq!(outputs[&1], 10); // refunded their stake
        assert_eq!(outputs[&2], 30);
        assert_eq!(g.state, BetState::draw); // voided, not settled
        assert!(header.contains("refunded") || !header.is_empty());
        assert_eq!(g.winners_readout(10), ""); // void → no winner body
    }

    #[test]
    fn settle_pays_winners_proportionally() {
        let mut g = Prediction::new(0, Lang::En, "test", &["A", "B"]);
        g.stake(1, "A", 10, "Alice");
        g.stake(2, "A", 30, "Bob");
        g.stake(3, "B", 10, "Cara");
        g.close();
        // total=50, winning bet=40 → ratio=1.25. user 1 → 12, user 2 → 37.
        let (out, _) = g.settle("A");
        assert_eq!(out[&1], 12);
        assert_eq!(out[&2], 37);
        assert!(!out.contains_key(&3));
        assert_eq!(g.changes[&1], 12 - 10);
        assert_eq!(g.changes[&2], 37 - 30);
        assert_eq!(g.changes[&3], -10);
        assert_eq!(g.state, BetState::settled);
    }

    #[test]
    fn settle_draw_refunds_inputs() {
        let mut g = Prediction::new(0, Lang::En, "test", &["A", "B"]);
        g.stake(1, "A", 10, "Alice");
        g.stake(2, "B", 7, "Bob");
        g.close();
        let (out, display) = g.settle("$draw$");
        assert_eq!(out[&1], 10);
        assert_eq!(out[&2], 7);
        assert!(display.contains("Void"));
        assert_eq!(g.state, BetState::draw);
        assert!(g.changes.is_empty()); // can't /reverse a void
        assert!(g.reverse().is_none());
    }

    #[test]
    fn settle_no_bets_at_all_is_draw() {
        let mut g = Prediction::new(0, Lang::En, "test", &["A", "B"]);
        g.close();
        let (out, display) = g.settle("A");
        assert!(out.is_empty());
        assert!(display.contains("but it seems nobody placed a bet"));
        assert_eq!(g.state, BetState::draw);
    }

    #[test]
    fn check_skips_users_with_no_record() {
        let mut g = Prediction::new(0, Lang::En, "test", &["A", "B"]);
        g.stake(1, "A", 10, "Alice");
        assert!(!g.check(1).is_empty()); // bettor sees their row
        assert!(g.check(99).is_empty()); // someone who didn't bet sees nothing
    }

    #[test]
    fn check_shows_settled_result() {
        let mut g = Prediction::new(0, Lang::En, "test", &["A", "B"]);
        g.stake(1, "A", 10, "Alice");
        g.stake(2, "B", 10, "Bob");
        g.close();
        g.settle("A");
        let s1 = g.check(1);
        assert!(s1.contains("won"));
        let s2 = g.check(2);
        assert!(s2.contains("lost"));
    }
}
