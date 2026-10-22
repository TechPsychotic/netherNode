//! Short invite codes: four everyday words, easy to read out loud and type.
//!
//! A full join code (host id + secret) is too long to say over a call. When hosting, a random
//! four-word code is picked and the full code is published under a key derived from those
//! words on iroh's public lookup service (the same one iroh uses to find computers). A friend
//! types the words, netherNode derives the same key, fetches the full code and joins with it.
//!
//! Only someone who has the words can find the record, and the words can't be guessed in
//! practice (2^44 combinations, each needing a lookup). The host still approves every friend.
//! Full codes keep working everywhere a short code does.

use std::time::Duration;

use iroh::SecretKey;
use iroh_dns::pkarr::SignedPacket;
use sha2::{Digest, Sha256};

use crate::error::{Result, other};
use crate::share::JoinCode;

const WORDS: &str = include_str!("share-words.txt");
const COUNT: usize = 4;
const RELAY: &str = "https://dns.iroh.link/pkarr";
const RECORD: &str = "_nethernode";
/// Records are refreshed while hosting so they don't expire mid-session.
pub(crate) const REPUBLISH: Duration = Duration::from_secs(20 * 60);

fn words() -> impl Iterator<Item = &'static str> {
    WORDS.lines().map(str::trim).filter(|w| !w.is_empty())
}

/// A four-word code, e.g. `maple-orbit-tunnel-frog`.
#[derive(Clone, PartialEq, Eq)]
pub struct ShortCode(Vec<&'static str>);

impl std::fmt::Display for ShortCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.join("-"))
    }
}

impl std::fmt::Debug for ShortCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // It grants a lookup of the full code; never print it by accident.
        f.write_str("ShortCode(…)")
    }
}

impl ShortCode {
    pub fn random() -> Result<Self> {
        let list: Vec<&'static str> = words().collect();
        let mut picks = Vec::with_capacity(COUNT);
        for _ in 0..COUNT {
            let mut b = [0u8; 2];
            getrandom::fill(&mut b).map_err(|e| other(format!("no secure random numbers: {e}")))?;
            // 2048 words: 11 bits, so no modulo bias.
            picks.push(list[usize::from(u16::from_be_bytes(b)) % list.len()]);
        }
        Ok(Self(picks))
    }

    /// Accepts spaces, dashes or dots, any case, and the first four letters of each word
    /// (every word in the list is unique by its first four letters).
    pub fn parse(s: &str) -> Option<Self> {
        let typed: Vec<String> = s
            .split(|c: char| c.is_whitespace() || c == '-' || c == '.' || c == ',')
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect();
        if typed.len() != COUNT {
            return None;
        }
        let picks = typed
            .iter()
            .map(|t| {
                // Exact word, else the word sharing its first four letters (forgives typos later on).
                words().find(|w| w == t).or_else(|| {
                    let start = t.get(..4).filter(|_| t.is_ascii())?;
                    words().find(|w| w.len() >= 4 && w.starts_with(start))
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self(picks))
    }

    fn key(&self) -> SecretKey {
        let mut hash = Sha256::new();
        hash.update(b"nethernode/short-invite/1\0");
        hash.update(self.to_string().as_bytes());
        SecretKey::from_bytes(&hash.finalize().into())
    }
}

/// Publish `code` under `short` on the lookup service.
pub(crate) async fn publish(
    http: &reqwest::Client,
    short: &ShortCode,
    code: &JoinCode,
) -> Result<()> {
    let key = short.key();
    let packet = SignedPacket::from_txt_strings(&key, RECORD, [code.to_string()], 3600)
        .map_err(|e| other(format!("couldn't make a short code: {e}")))?;
    http.put(format!("{RELAY}/{}", key.public().to_z32()))
        .body(packet.to_relay_payload())
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| other(format!("couldn't publish the short code: {e}")))?;
    Ok(())
}

/// Look up the full code behind a short one.
async fn lookup(http: &reqwest::Client, short: &ShortCode) -> Result<JoinCode> {
    let public = short.key().public();
    let not_found =
        || other("no one is sharing with that code right now; check the words with your friend");
    let response = http
        .get(format!("{RELAY}/{}", public.to_z32()))
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| other(format!("couldn't look up the code: {e}")))?;
    if !response.status().is_success() {
        return Err(not_found());
    }
    let payload = response
        .bytes()
        .await
        .map_err(|e| other(format!("couldn't look up the code: {e}")))?;
    let packet = SignedPacket::from_relay_payload(&public, &payload).map_err(|_| not_found())?;
    packet
        .txt_records(RECORD)
        .iter()
        .find_map(|t| t.parse().ok())
        .ok_or_else(not_found)
}

/// Whether `s` looks like a code at all (short or full), for enabling the Join button.
pub fn looks_valid(s: &str) -> bool {
    ShortCode::parse(s).is_some() || s.parse::<JoinCode>().is_ok()
}

/// Turn what a friend typed (four words or a full code) into a full join code.
pub async fn resolve(s: &str) -> Result<JoinCode> {
    if let Ok(code) = s.parse::<JoinCode>() {
        return Ok(code);
    }
    let short = ShortCode::parse(s).ok_or_else(|| {
        other("that isn't a netherNode code: it should be four words, like maple-orbit-tunnel-frog")
    })?;
    lookup(&crate::download::client(), &short).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_codes_parse_loosely() {
        assert_eq!(words().count(), 2048);
        let code = ShortCode::random().unwrap();
        let text = code.to_string();
        assert_eq!(ShortCode::parse(&text), Some(code.clone()));
        assert_eq!(
            ShortCode::parse(&text.replace('-', " ").to_uppercase()),
            Some(code.clone())
        );
        // First four letters are enough.
        let abbreviated: Vec<String> = code.0.iter().map(|w| w.chars().take(4).collect()).collect();
        assert_eq!(ShortCode::parse(&abbreviated.join(" ")), Some(code));
        assert_eq!(ShortCode::parse("abandon ability able"), None);
        assert_eq!(ShortCode::parse("abandon ability able zzzz"), None);
        assert!(
            ShortCode::parse("abandon ability able about")
                .unwrap()
                .key()
                .public()
                != ShortCode::parse("abandon ability able above")
                    .unwrap()
                    .key()
                    .public()
        );
    }

    /// Uses the real lookup service: `cargo test -p nethernode-core -- --ignored invite`.
    #[tokio::test]
    #[ignore]
    async fn publishes_and_resolves_over_the_internet() {
        let host = SecretKey::from_bytes(&[7; 32]).public();
        let code: JoinCode = format!("{}-0011223344556677", host.to_z32())
            .parse()
            .unwrap();
        let short = ShortCode::random().unwrap();
        publish(&crate::download::client(), &short, &code)
            .await
            .unwrap();
        assert_eq!(resolve(&short.to_string()).await.unwrap(), code);
        assert!(resolve("abandon ability able about").await.is_err());
    }
}
