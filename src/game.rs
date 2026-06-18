use crate::i18n::{self, Lang};
use crate::types::BetState;
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
    pub odd: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BetGame {
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
}

impl BetGame {
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
            BetState::draw => "🤝",
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
        for (i, opt) in self.option_order.iter().enumerate() {
            if let Some(d) = self.options.get(opt) {
                let mult = if d.bet > 0 {
                    format!("×{}", d.odd)
                } else {
                    "×—".to_string()
                };
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
                // Settle: all outcome buttons on one row, draw on its own row.
                rows.push(
                    self.option_order
                        .iter()
                        .map(|opt| (opt.clone(), format!("gamble:{opt}")))
                        .collect(),
                );
                rows.push(vec![(
                    i18n::draw_label(self.lang).to_string(),
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
        // Recompute the odd for *every* option that has bets — total moved,
        // so all displayed odds are now stale. Mirrors the Python original.
        let total = self.total;
        for d in self.options.values_mut() {
            if d.bet > 0 {
                let odd = (total as f64 / d.bet as f64 * 1000.0).round() / 1000.0;
                d.odd = format!("{odd:.2}");
            }
        }
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
                i18n::result_header(self.lang, i18n::draw_label(self.lang)),
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
        let mut changes = HashMap::new();
        for (&user, &input) in &self.inputs {
            let win = outputs.get(&user).copied().unwrap_or(0);
            changes.insert(user, win - input);
        }
        self.outputs = outputs.clone();
        self.changes = changes.clone();

        if changes.is_empty() {
            // Nobody bet at all.
            self.state = BetState::draw;
            let display = i18n::result_header(self.lang, outcome)
                + i18n::no_one_bet_suffix(self.lang);
            return (outputs, display);
        }

        self.state = BetState::settled;
        let mut display = i18n::result_header(self.lang, outcome);
        for (user, diff) in &changes {
            let name = self.display_name(*user);
            let verb = if *diff >= 0 {
                i18n::verb_won(self.lang)
            } else {
                i18n::verb_lost(self.lang)
            };
            display.push_str(&i18n::settle_line(
                self.lang,
                &name,
                verb,
                &diff.abs().to_string(),
            ));
        }
        (outputs, display)
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
    /// game, so callers can skip uninteresting entries when listing.
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
        let mut g = BetGame::new(0, Lang::En, "test", &["A", "B"]);
        g.stake(1, "A", 10, "Alice");
        g.stake(2, "B", 5, "Bob");
        // total = 15. A.bet = 10 → odd = 1.50. B.bet = 5 → odd = 3.00.
        assert_eq!(g.options["A"].odd, "1.50");
        assert_eq!(g.options["B"].odd, "3.00");
        // Another bet on A should update B's odd too.
        g.stake(3, "A", 15, "Cara");
        // total = 30. A.bet = 25 → odd = 1.20. B.bet = 5 → odd = 6.00.
        assert_eq!(g.options["A"].odd, "1.20");
        assert_eq!(g.options["B"].odd, "6.00");
    }

    #[test]
    fn settle_pays_winners_proportionally() {
        let mut g = BetGame::new(0, Lang::En, "test", &["A", "B"]);
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
        let mut g = BetGame::new(0, Lang::En, "test", &["A", "B"]);
        g.stake(1, "A", 10, "Alice");
        g.stake(2, "B", 7, "Bob");
        g.close();
        let (out, display) = g.settle("$draw$");
        assert_eq!(out[&1], 10);
        assert_eq!(out[&2], 7);
        assert!(display.contains("Draw"));
        assert_eq!(g.state, BetState::draw);
        assert!(g.changes.is_empty()); // can't /reverse a draw
        assert!(g.reverse().is_none());
    }

    #[test]
    fn settle_no_bets_at_all_is_draw() {
        let mut g = BetGame::new(0, Lang::En, "test", &["A", "B"]);
        g.close();
        let (out, display) = g.settle("A");
        assert!(out.is_empty());
        assert!(display.contains("but it seems nobody placed a bet"));
        assert_eq!(g.state, BetState::draw);
    }

    #[test]
    fn check_skips_users_with_no_record() {
        let mut g = BetGame::new(0, Lang::En, "test", &["A", "B"]);
        g.stake(1, "A", 10, "Alice");
        assert!(!g.check(1).is_empty()); // bettor sees their row
        assert!(g.check(99).is_empty()); // someone who didn't bet sees nothing
    }

    #[test]
    fn check_shows_settled_result() {
        let mut g = BetGame::new(0, Lang::En, "test", &["A", "B"]);
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
