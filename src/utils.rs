use std::collections::HashSet;

pub fn string_sum(s: &str) -> i32 {
    s.chars().map(|c| c as i32).sum()
}

pub fn string_hash(target: &str, base: &str) -> i32 {
    let n: u32 = 13;
    let m: i32 = 1999;
    let t = string_sum(target);
    let b = string_sum(base);
    ((t - b) % m).wrapping_pow(n) % m
}

pub fn old_determine(list: &[String]) -> Option<String> {
    if list.is_empty() {
        return None;
    }
    let mods: Vec<i32> = list.iter().map(|s| string_sum(s) % 100).collect();
    let total = mods.iter().sum::<i32>() % 100;
    let mut min_diff = i32::MAX;
    let mut min_idx = 0;
    for (i, &m) in mods.iter().enumerate() {
        let diff = (m - total).abs();
        if diff < min_diff {
            min_diff = diff;
            min_idx = i;
        }
    }
    Some(list[min_idx].clone())
}

pub fn determine(opts: &[String], question: &str) -> Option<String> {
    if opts.is_empty() {
        return None;
    }
    let mut best_idx = 0;
    let mut best = i32::MAX;
    for (i, opt) in opts.iter().enumerate() {
        let h = string_hash(opt, question).abs();
        if h < best {
            best = h;
            best_idx = i;
        }
    }
    Some(opts[best_idx].clone())
}

pub fn determine_all(opts: &[String], question: &str) -> Vec<String> {
    let mut pairs: Vec<(i32, &String)> = opts
        .iter()
        .map(|o| (string_hash(o, question).abs(), o))
        .collect();
    pairs.sort_by_key(|(h, _)| *h);
    pairs.into_iter().map(|(_, s)| s.clone()).collect()
}

pub fn split_question(input: &str) -> Option<(String, String)> {
    let q = if input.contains('？') {
        '？'
    } else if input.contains('?') {
        '?'
    } else {
        return None;
    };
    let mut parts = input.splitn(2, q);
    let head = parts.next()?.trim();
    let tail = parts.next()?.trim();
    if head.is_empty() || tail.is_empty() {
        return None;
    }
    Some((head.to_string(), tail.to_string()))
}

pub fn wolfram_replace(s: &str) -> String {
    s.replace('+', "%2B")
        .replace(',', "%2C")
        .replace('=', "%3D")
        .replace('/', "%2F")
}

pub fn cloth_check(cloth: &str) -> (bool, String) {
    let mut seen = HashSet::new();
    let mut penalty = false;
    let mut remain = String::new();
    for ch in cloth.chars() {
        if !seen.insert(ch) {
            penalty = true;
        } else {
            remain.push(ch);
        }
    }
    (penalty, remain)
}
