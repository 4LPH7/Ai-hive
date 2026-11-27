//! Shared word splitting. A leaf used by ace selection, graph search and index; miner and skill keep their own
//! rules because their charset and stopword lists differ.

/// Lowercased ASCII-alphanumeric words of at least `min_len` bytes, in order.
pub fn alnum_words(text: &str, min_len: usize) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() >= min_len)
        .map(str::to_lowercase)
        .collect()
}
