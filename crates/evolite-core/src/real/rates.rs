//! Settings → "Fiat rate": the price of DASH from the service DashPay for Android
//! uses (CTX, `rates.ctx.com`), fetched over https when asked and every 30 minutes after. The
//! service sees our IP; the setting is off until the person turns it on, and says so.
//!
//! Testnet coins are worth nothing: the screens show the mainnet price as an example.

use std::time::Duration;

/// DashPay for Android: `DashRetailClient` ("rates?source=ctx").
pub const URL: &str = "https://rates.ctx.com/rates?source=ctx";
/// DashPay keeps a price for 30 minutes before it is stale.
pub const EVERY: Duration = Duration::from_secs(30 * 60);
/// The answer is about 30 KB; anything far bigger is not it.
const MAX_BYTES: u64 = 512 * 1024;

/// The price of one DASH in `currency` (hundred-millionths of it) from the service's answer: a
/// list of `{ baseCurrency, quoteCurrency, price }` with prices as decimal strings.
pub fn price_in(answer: &[u8], currency: &str) -> Result<i64, String> {
    let list: Vec<serde_json::Value> = serde_json::from_slice(answer)
        .map_err(|_| tr!("the rate service did not answer with a list of rates"))?;
    let price = list
        .iter()
        .find(|r| r["baseCurrency"] == "DASH" && r["quoteCurrency"] == currency)
        .and_then(|r| r["price"].as_str())
        .ok_or_else(|| {
            format!(
                tr!("the service has no DASH rate in {currency}"),
                currency = currency
            )
        })?;
    decimal(price).ok_or_else(|| {
        format!(
            tr!("the service sent an unreadable rate «{price}»"),
            price = price
        )
    })
}

/// "47.9615" → 4 796 150 000: up to 8 decimals, no floating point, nothing negative.
fn decimal(s: &str) -> Option<i64> {
    let (whole, fraction) = s.split_once('.').unwrap_or((s, ""));
    let digits = |t: &str| !t.is_empty() && t.chars().all(|c| c.is_ascii_digit());
    if !digits(whole) || (!fraction.is_empty() && !digits(fraction)) {
        return None;
    }
    let fraction: String = fraction.chars().take(8).collect();
    let fraction: i64 = format!("{fraction:0<8}").parse().ok()?;
    whole
        .parse::<i64>()
        .ok()?
        .checked_mul(100_000_000)?
        .checked_add(fraction)
}

/// Asks the service; blocking (ureq), so the core runs it off its own tasks.
pub fn fetch(currency: &str) -> Result<i64, String> {
    use std::io::Read;
    let config = ureq::Agent::config_builder()
        .https_only(true)
        // Straight to the service, never a proxy from the environment: evolite has no proxy
        // setting, and one set for other programs is not the person's choice for it.
        .proxy(None)
        .max_redirects(0)
        .timeout_global(Some(Duration::from_secs(20)))
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut response = agent.get(URL).call().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    price_in(&bytes, currency)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSWER: &[u8] = br#"[
        {"baseCurrency":"BCH","price":"406.4428","quoteCurrency":"USD","retrieved":"2026-10-31T13:11:00Z","source":"ctx-average","symbol":"BCHUSD"},
        {"baseCurrency":"DASH","price":"47.9615","quoteCurrency":"USD","retrieved":"2026-10-31T13:17:50Z","source":"ctx-average","symbol":"DASHUSD"},
        {"baseCurrency":"DASH","price":"3812.123456789","quoteCurrency":"RUB","retrieved":"2026-10-31T13:17:50Z","source":"ctx-average","symbol":"DASHRUB"}
    ]"#;

    #[test]
    fn finds_dash_in_the_currency() {
        assert_eq!(price_in(ANSWER, "USD"), Ok(4_796_150_000));
        // Beyond 8 decimals is cut, not rounded through a float.
        assert_eq!(price_in(ANSWER, "RUB"), Ok(381_212_345_678));
    }

    #[test]
    fn says_what_is_wrong() {
        assert!(price_in(ANSWER, "EUR").unwrap_err().contains("EUR"));
        assert!(price_in(b"<html>", "USD").is_err());
        assert!(
            price_in(
                br#"[{"baseCurrency":"DASH","quoteCurrency":"USD","price":"-1"}]"#,
                "USD"
            )
            .is_err()
        );
        assert!(
            price_in(
                br#"[{"baseCurrency":"DASH","quoteCurrency":"USD","price":"1e3"}]"#,
                "USD"
            )
            .is_err()
        );
    }
}
