//! Token estimation for prompt-budgeting purposes only.
//!
//! Nothing here is a real tokenizer. It exists so the assembler can decide
//! what fits in a window *before* we hand text to a provider's actual
//! tokenizer/encoder. Always re-count (or let the provider count) with the
//! real tokenizer immediately before a request goes out — these numbers are
//! for budgeting and trimming decisions inside this crate, not for billing
//! or for hard limits enforced against a live API.
use std::collections::HashMap;
use std::sync::Mutex;

/// Anything that can estimate a token count for a piece of text.
pub trait TokenCounter: Send + Sync {
    fn count(&self, text: &str) -> i64;
}

/// A character-class-aware heuristic counter.
///
/// `len/4` (bytes-over-four) is the usual shortcut, and it is badly wrong for
/// Thai: Thai has no spaces between words, and — because it's encoded as
/// 3-byte UTF-8 sequences per character — a byte-length/4 estimate divides
/// by *bytes*, not characters, and ends up wildly undercounting how many
/// tokens the run will actually cost. This counter instead classifies runs
/// of characters and weighs each class separately:
///
/// - ASCII / Latin text: ~4 characters per token (the common English/BPE
///   rule of thumb — mixed short/long words, whitespace, punctuation).
/// - CJK (Chinese/Japanese/Korean) ideographs and kana: ~1 character per
///   token — byte-pair encoders over CJK text tend to land close to one
///   token per character because there are no multi-character "common
///   word" merges the way there are in space-delimited scripts.
/// - Thai (`\u{0E00}..=\u{0E7F}`): ~1 token per character, the same density
///   as CJK. Thai is 3 bytes/char in UTF-8, has no word-separating spaces,
///   and — despite being alphabetic rather than one-glyph-per-morpheme —
///   is thin enough in most byte-level BPE vocabularies that merges rarely
///   span more than a character or two; in practice Thai often tokenizes
///   *worse* than CJK on general-purpose tokenizers. Treating it as roughly
///   one token per character is the defensible middle, and — critically —
///   it is nowhere near "bytes/4": at 3 bytes/char, "bytes/4" implies
///   ~1.33 characters per token, i.e. it silently assumes Thai is *less*
///   dense per byte than plain ASCII, when real tokenizers show the
///   opposite. That inversion is exactly the len/4 bug this type exists to
///   avoid.
/// - Everything else (other scripts, symbols, emoji): ~2 characters per
///   token, a conservative middle ground.
///
/// Runs are counted in characters, not bytes, and each run's token count is
/// rounded up so that any non-empty run costs at least one token.
pub struct HeuristicCounter;

#[derive(Clone, Copy, PartialEq, Eq)]
enum CharClass {
    AsciiLatin,
    Cjk,
    Thai,
    Other,
}

fn classify(c: char) -> CharClass {
    if c.is_ascii() || matches!(c, '\u{00C0}'..='\u{024F}') {
        // ASCII, or Latin-1 supplement / Latin extended-A/B.
        CharClass::AsciiLatin
    } else if matches!(c,
        '\u{4E00}'..='\u{9FFF}'   // CJK unified ideographs
        | '\u{3040}'..='\u{30FF}' // Hiragana/Katakana
        | '\u{AC00}'..='\u{D7A3}' // Hangul syllables
        | '\u{3400}'..='\u{4DBF}' // CJK ext A
    ) {
        CharClass::Cjk
    } else if matches!(c, '\u{0E00}'..='\u{0E7F}') {
        CharClass::Thai
    } else {
        CharClass::Other
    }
}

/// Characters per token, by class. A larger value means fewer tokens per
/// character (the class packs more densely into each token).
const ASCII_CHARS_PER_TOKEN: f64 = 4.0;
const CJK_CHARS_PER_TOKEN: f64 = 1.0;
const THAI_CHARS_PER_TOKEN: f64 = 1.0;
const OTHER_CHARS_PER_TOKEN: f64 = 2.0;

fn chars_per_token(class: CharClass) -> f64 {
    match class {
        CharClass::AsciiLatin => ASCII_CHARS_PER_TOKEN,
        CharClass::Cjk => CJK_CHARS_PER_TOKEN,
        CharClass::Thai => THAI_CHARS_PER_TOKEN,
        CharClass::Other => OTHER_CHARS_PER_TOKEN,
    }
}

impl TokenCounter for HeuristicCounter {
    fn count(&self, text: &str) -> i64 {
        if text.is_empty() {
            return 0;
        }
        // Accumulate run lengths per class, then convert each run to tokens
        // (rounded up) so a lone character still costs at least one token
        // and short runs don't vanish to zero under truncation.
        let mut total_tokens: i64 = 0;
        let mut run_class: Option<CharClass> = None;
        let mut run_len: usize = 0;

        fn flush(class: Option<CharClass>, len: usize, total: &mut i64) {
            if len == 0 {
                return;
            }
            if let Some(class) = class {
                let per = chars_per_token(class);
                let toks = (len as f64 / per).ceil() as i64;
                *total += toks.max(1);
            }
        }

        for c in text.chars() {
            let class = classify(c);
            match run_class {
                Some(rc) if rc == class => {
                    run_len += 1;
                }
                _ => {
                    flush(run_class, run_len, &mut total_tokens);
                    run_class = Some(class);
                    run_len = 1;
                }
            }
        }
        flush(run_class, run_len, &mut total_tokens);
        total_tokens
    }
}

/// A cheap, non-cryptographic content hash used only as a cache key (not
/// for security or content addressing — see `types::BlobId` for that).
fn hash_text(text: &str) -> u64 {
    // FNV-1a 64-bit: fast, dependency-free, good enough distribution for a
    // cache key.
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = FNV_OFFSET;
    for b in text.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// Wraps any `TokenCounter` with an in-memory cache keyed by a cheap content
/// hash, so repeated content (e.g. the same file body appearing again in a
/// window) is counted once. Interior-mutable so it works behind a shared
/// `&self`.
pub struct CachedCounter<C: TokenCounter> {
    inner: C,
    cache: Mutex<HashMap<u64, i64>>,
    hits: Mutex<u64>,
    misses: Mutex<u64>,
}

impl<C: TokenCounter> CachedCounter<C> {
    pub fn new(inner: C) -> Self {
        Self {
            inner,
            cache: Mutex::new(HashMap::new()),
            hits: Mutex::new(0),
            misses: Mutex::new(0),
        }
    }

    /// Number of `count` calls that were served from the cache.
    pub fn hits(&self) -> u64 {
        *self.hits.lock().unwrap()
    }

    /// Number of `count` calls that had to invoke the inner counter.
    pub fn misses(&self) -> u64 {
        *self.misses.lock().unwrap()
    }
}

impl<C: TokenCounter> TokenCounter for CachedCounter<C> {
    fn count(&self, text: &str) -> i64 {
        let key = hash_text(text);
        if let Some(&v) = self.cache.lock().unwrap().get(&key) {
            *self.hits.lock().unwrap() += 1;
            return v;
        }
        *self.misses.lock().unwrap() += 1;
        let v = self.inner.count(text);
        self.cache.lock().unwrap().insert(key, v);
        v
    }
}
