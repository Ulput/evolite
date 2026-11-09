//! What must never reach the journal or a report, masked in text that comes from outside the
//! code (a panic, a sync error): pieces of a phrase, keys, addresses, txids and ids, IP
//! addresses, `@names`, amounts, e-mail addresses and the user's name in paths.

/// What a masked piece becomes.
const MASK: &str = "‹…›";

/// BIP-39 words in a row from this many on are a piece of a phrase.
const PHRASE_RUN: usize = 3;

/// `text` with everything private masked.
pub fn scrub(text: &str) -> String {
    let text = user_in_paths(text);
    let mut out = String::with_capacity(text.len());
    let mut chunk = String::new();
    for c in text.chars() {
        if c.is_whitespace() {
            out.push_str(&scrub_chunk(&chunk));
            chunk.clear();
            out.push(c);
        } else {
            chunk.push(c);
        }
    }
    out.push_str(&scrub_chunk(&chunk));
    mask_phrase_runs(&out, PHRASE_RUN)
}

/// The longest run of BIP-39 words in a row in `text` (words split by anything but letters):
/// a report with many of them may hold the phrase.
pub fn phrase_words(text: &str) -> usize {
    let mut best = 0;
    let mut run = 0;
    for word in text
        .split(|c: char| !c.is_ascii_alphabetic())
        .filter(|w| !w.is_empty())
    {
        if is_bip39(word) {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

fn is_bip39(word: &str) -> bool {
    word.len() >= 3 && crate::bip39::index(&word.to_ascii_lowercase()).is_some()
}

/// `C:\Users\anna\…`, `/home/anna/…`, `/Users/anna/…`: the name masked.
fn user_in_paths(text: &str) -> String {
    const MARKERS: [&str; 5] = [r"\Users\", r"\users\", "/Users/", "/users/", "/home/"];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    // The nearest marker each time.
    while let Some((i, marker)) = MARKERS
        .iter()
        .filter_map(|m| rest.find(m).map(|i| (i, *m)))
        .min()
    {
        let after = i + marker.len();
        out.push_str(&rest[..after]);
        let end = rest[after..]
            .find(['\\', '/', '"', '\'', ' ', '\n', ':'])
            .map_or(rest.len(), |n| after + n);
        if end > after {
            out.push_str(MASK);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// One whitespace-free piece of text.
fn scrub_chunk(chunk: &str) -> String {
    if chunk.is_empty() {
        return String::new();
    }
    let core = chunk.trim_matches(|c: char| ",;()[]{}<>\"'`".contains(c) || c == '.');
    if let Some(at) = core.find('@') {
        // `@name`, or an e-mail address.
        if at == 0 || core[at..].contains('.') {
            return chunk.replace(core, MASK);
        }
    }
    if is_ip(core) {
        return chunk.replace(core, MASK);
    }
    // Inside the piece: long alphanumeric runs, hex runs and decimal numbers.
    let chars: Vec<char> = chunk.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_alphanumeric() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i].is_ascii_alphanumeric() {
            i += 1;
        }
        let run: String = chars[start..i].iter().collect();
        // A decimal number: digits, a point or a comma, digits.
        if run.chars().all(|c| c.is_ascii_digit())
            && i + 1 < chars.len()
            && matches!(chars[i], '.' | ',')
            && chars[i + 1].is_ascii_digit()
        {
            i += 1;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            out.push_str(MASK);
            continue;
        }
        if is_secret_run(&run) {
            out.push_str(MASK);
        } else {
            out.push_str(&run);
        }
    }
    out
}

/// Base58 or bech32 strings (addresses, ids, WIF keys) and hex (txids, hashes, keys). Base58
/// has nine digits in 58 characters: about one address in 300 has none, so a run as long as an
/// address (25 or more) goes without one too.
fn is_secret_run(run: &str) -> bool {
    let hex = run.len() >= 16 && run.chars().all(|c| c.is_ascii_hexdigit());
    let long = run.len() >= 25 || (run.len() >= 20 && run.chars().any(|c| c.is_ascii_digit()));
    let bech32 = (run.starts_with("tdash1") || run.starts_with("dash1")) && run.len() > 10;
    hex || long || bech32
}

/// An IPv4 or IPv6 address, with a port or not, in brackets or not.
fn is_ip(text: &str) -> bool {
    let text = text.trim_start_matches('[');
    let host = match text.rsplit_once("]:") {
        Some((host, _)) => host,
        None => text.trim_end_matches(']'),
    };
    if host.parse::<std::net::Ipv6Addr>().is_ok() {
        return true;
    }
    let v4 = host.rsplit_once(':').map_or(host, |(h, port)| {
        if port.chars().all(|c| c.is_ascii_digit()) {
            h
        } else {
            host
        }
    });
    v4.parse::<std::net::Ipv4Addr>().is_ok()
}

/// Runs of `min` BIP-39 words and more, separated by spaces or punctuation only, masked.
fn mask_phrase_runs(text: &str, min: usize) -> String {
    // Words with where they are.
    let mut words: Vec<(usize, usize)> = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        match (c.is_ascii_alphabetic(), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                words.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        words.push((s, text.len()));
    }
    let mut masked: Vec<(usize, usize)> = Vec::new();
    let mut run: Vec<(usize, usize)> = Vec::new();
    let mut flush = |run: &mut Vec<(usize, usize)>| {
        if run.len() >= min {
            masked.push((run[0].0, run[run.len() - 1].1));
        }
        run.clear();
    };
    for &(s, e) in &words {
        let joined = run
            .last()
            .is_none_or(|&(_, last)| text[last..s].chars().all(|c| !c.is_alphanumeric()));
        if !joined {
            flush(&mut run);
        }
        if is_bip39(&text[s..e]) {
            run.push((s, e));
        } else {
            flush(&mut run);
        }
    }
    flush(&mut run);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (s, e) in masked {
        out.push_str(&text[at..s]);
        out.push_str(MASK);
        at = e;
    }
    out.push_str(&text[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gone(text: &str, secret: &str) {
        let out = scrub(text);
        assert!(!out.contains(secret), "{secret} in {out}");
        assert!(out.contains(MASK), "{out}");
    }

    #[test]
    fn masks_pieces_of_a_phrase() {
        gone(
            "the words were abandon ability able about above absent",
            "ability able about",
        );
        // A word or two of plain English stays.
        assert_eq!(scrub("thread main panicked"), "thread main panicked");
        assert_eq!(phrase_words("abandon, ability; able about above"), 5);
        assert_eq!(phrase_words("Το παράθυρο δεν ανοίγει"), 0);
    }

    #[test]
    fn masks_addresses_ids_keys_and_txids() {
        gone("pay yMrDQWqicXKL8xdJg23oYVVqKNtm5AwvXQ now", "yMrDQ");
        gone(
            "identity F5teLR3Vq4JjzQA7jgdGjZQvJbKpMubbu5dsC4MJd1Eh",
            "F5teLR3",
        );
        gone("tdash1qz8f5v6tnhx0ldqpw9c3ujmtk2yg7s4e", "tdash1q");
        gone(
            "txid 4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b",
            "4a5e1e4b",
        );
        gone(
            "wif cVt4o7BGAig1UXywgGSmARhxMdzP5qvQsxKkSsc1XEkw3tDTQFpy",
            "cVt4o7",
        );
        gone("hash deadbeefcafebabe", "deadbeef");
        // Base58 without a single digit: an address and an identity id.
        gone("pay yMrDQWqicXKLxdJgoYVVqKNtmAwvXQaBcD now", "yMrDQ");
        gone("identity FteLRVqJjzQAjgdGjZQvJbKpMubbudsCMJdEhab", "FteLR");
        // Ordinary words and code paths stay readable.
        assert_eq!(
            scrub("thread main panicked at evolite_core::real::chain"),
            "thread main panicked at evolite_core::real::chain"
        );
    }

    #[test]
    fn masks_ip_addresses() {
        gone("peer 192.168.1.10:19999 disconnected", "192.168");
        gone("peer [2001:db8::1]:19999 gone", "2001:db8");
        gone("node fe80::1 refused", "fe80::1");
        gone("(10.0.0.2)", "10.0.0.2");
    }

    #[test]
    fn masks_names_amounts_and_mail() {
        gone("sent to @bob", "bob");
        gone("amount 0.25 DASH", "0.25");
        gone("ποσό 12,846", "12,846");
        gone("write to anna@example.com", "anna@");
    }

    #[test]
    fn masks_the_user_in_paths() {
        gone(
            r"at C:\Users\Ελένη\AppData\Roaming\evolite\wallet.json",
            "Ελένη",
        );
        gone("open /home/anna/.local/share/evolite failed", "anna");
        gone("/Users/anna/Library/evolite", "anna");
        assert!(scrub(r"C:\Users\Ελένη\x").contains(r"C:\Users\"));
    }

    #[test]
    fn leaves_what_a_report_needs() {
        let text = "src/app.rs:120:15 index out of bounds: the len is 3 but the index is 7";
        assert_eq!(scrub(text), text);
        assert_eq!(
            scrub("stage Filters height 1284112"),
            "stage Filters height 1284112"
        );
    }
}
