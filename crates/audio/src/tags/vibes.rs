//! Mirroring vibes into a Comment as `#slug` hashtags.
//!
//! Overtone owns only its own tokens: hashtags whose slug is a vibe in its
//! database. Everything else in the Comment (Mixed In Key key and energy
//! text, Rekordbox's `/* tag / tag */` My Tag block, emoji, hashtags from
//! other tools) is left exactly as it is. See
//! `engineering/decisions/vibe-tags.md`.

use std::collections::HashSet;
use std::hash::BuildHasher;
use std::ops::Range;

use overtone_model::VibePath;

/// Whitespace-delimited tokens with their byte ranges.
fn tokens(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = None;
    for (index, c) in text.char_indices() {
        match (c.is_whitespace(), start) {
            (true, Some(s)) => {
                out.push(s..index);
                start = None;
            }
            (false, None) => start = Some(index),
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push(s..text.len());
    }
    out
}

/// The lowercase slug of a hashtag token, or `None` if it isn't one.
fn token_slug(token: &str) -> Option<String> {
    let slug = token.strip_prefix('#')?;
    (!slug.is_empty() && !slug.starts_with('#')).then(|| slug.to_lowercase())
}

/// Rewrites `existing` so its Overtone hashtags match `desired`.
///
/// - Removes `#slug` tokens whose slug is in `known_slugs` but not desired,
///   together with the whitespace that separated them from the text before
///   (or after, for a leading token).
/// - Appends missing desired hashtags at the end, separated by single spaces.
/// - Leaves every other character untouched.
/// - Matches slugs case-insensitively; a token boundary is whitespace or the
///   start or end of the text.
/// - Idempotent: applying the result again returns it unchanged.
///
/// Returns `None` when the result would be empty or whitespace only.
pub fn apply_vibes_to_comment<S: BuildHasher>(
    existing: Option<&str>,
    known_slugs: &HashSet<String, S>,
    desired: &[VibePath],
) -> Option<String> {
    let text = existing.unwrap_or("");
    let known: HashSet<String> = known_slugs.iter().map(|s| s.to_lowercase()).collect();
    let mut wanted: Vec<String> = Vec::new();
    for vibe in desired {
        let slug = vibe.slug();
        if !wanted.contains(&slug) {
            wanted.push(slug);
        }
    }

    let spans = tokens(text);
    let mut remove: Vec<Range<usize>> = Vec::new();
    let mut present: HashSet<String> = HashSet::new();
    let mut kept_before = false;
    for (i, span) in spans.iter().enumerate() {
        let slug = token_slug(&text[span.clone()]);
        match slug {
            Some(slug) if !wanted.contains(&slug) && known.contains(&slug) => {
                // Take the whitespace that separated the token from kept
                // text before it; with nothing kept before, take the
                // whitespace after it instead.
                let cut = if kept_before {
                    spans[i - 1].end..span.end
                } else {
                    let next_start = spans.get(i + 1).map_or(text.len(), |next| next.start);
                    span.start..next_start
                };
                remove.push(cut);
            }
            Some(slug) => {
                if wanted.contains(&slug) {
                    present.insert(slug);
                }
                kept_before = true;
            }
            None => kept_before = true,
        }
    }

    let mut result = String::with_capacity(text.len() + 32);
    let mut pos = 0;
    for cut in &remove {
        let start = cut.start.max(pos);
        result.push_str(&text[pos..start]);
        pos = pos.max(cut.end);
    }
    result.push_str(&text[pos..]);

    for slug in &wanted {
        if present.contains(slug) {
            continue;
        }
        if !result.is_empty() && !result.ends_with(char::is_whitespace) {
            result.push(' ');
        }
        result.push('#');
        result.push_str(slug);
    }

    (!result.trim().is_empty()).then_some(result)
}

/// Hashtag slugs found in `text` (lowercase, in order, without duplicates),
/// for offering to import a Comment or Grouping's hashtags as vibes.
/// Trailing sentence punctuation (`,;.!?`) is not part of the slug.
pub fn hashtags_in(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for span in tokens(text) {
        let token = text[span].trim_end_matches([',', ';', '.', '!', '?']);
        if let Some(slug) = token_slug(token)
            && !out.contains(&slug)
        {
            out.push(slug);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vibes(paths: &[&str]) -> Vec<VibePath> {
        paths
            .iter()
            .map(|p| VibePath::parse(p).expect("vibe"))
            .collect()
    }

    fn known(slugs: &[&str]) -> HashSet<String> {
        slugs.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn appends_to_mixed_in_key_text() {
        let out = apply_vibes_to_comment(
            Some("8A - Energy 6"),
            &known(&["warmup", "dubs/deep"]),
            &vibes(&["Warmup", "Dubs/Deep"]),
        );
        assert_eq!(out.as_deref(), Some("8A - Energy 6 #warmup #dubs/deep"));
    }

    #[test]
    fn removes_only_known_undesired_tokens() {
        let existing = "8A - Energy 6 #warmup #other-tool 🔥 #dubs /* House / Deep */";
        let out = apply_vibes_to_comment(
            Some(existing),
            &known(&["warmup", "dubs"]),
            &vibes(&["Dubs"]),
        );
        assert_eq!(
            out.as_deref(),
            Some("8A - Energy 6 #other-tool 🔥 #dubs /* House / Deep */")
        );
    }

    #[test]
    fn matches_case_insensitively_and_keeps_existing_spelling() {
        let out = apply_vibes_to_comment(
            Some("#WarmUp note"),
            &known(&["warmup"]),
            &vibes(&["Warmup"]),
        );
        assert_eq!(out.as_deref(), Some("#WarmUp note"));
        let out = apply_vibes_to_comment(Some("#WARMUP note"), &known(&["warmup"]), &[]);
        assert_eq!(out.as_deref(), Some("note"));
    }

    #[test]
    fn prefix_slugs_are_distinct_tokens() {
        let out = apply_vibes_to_comment(
            Some("#dubtechno"),
            &known(&["dub", "dubtechno"]),
            &vibes(&["Dubtechno", "Dub"]),
        );
        assert_eq!(out.as_deref(), Some("#dubtechno #dub"));
        // A hashtag glued to other text is not a token.
        let out = apply_vibes_to_comment(Some("x#dub"), &known(&["dub"]), &[]);
        assert_eq!(out.as_deref(), Some("x#dub"));
    }

    #[test]
    fn empty_results_are_none() {
        assert_eq!(apply_vibes_to_comment(None, &known(&[]), &[]), None);
        assert_eq!(
            apply_vibes_to_comment(Some("#warmup"), &known(&["warmup"]), &[]),
            None
        );
        assert_eq!(
            apply_vibes_to_comment(None, &known(&[]), &vibes(&["Deep Cuts"])).as_deref(),
            Some("#deep-cuts")
        );
    }

    #[test]
    fn leading_token_removal_takes_following_space() {
        let out = apply_vibes_to_comment(Some("#warmup  8A\tEnergy"), &known(&["warmup"]), &[]);
        assert_eq!(out.as_deref(), Some("8A\tEnergy"));
    }

    #[test]
    fn finds_hashtags_for_import() {
        assert_eq!(
            hashtags_in("Nice #Warmup, #dubs/deep #warmup # ## x#y"),
            vec!["warmup".to_string(), "dubs/deep".to_string()]
        );
    }

    /// Small deterministic generator so the property test needs no dependency.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
            &items[usize::try_from(self.next() % items.len() as u64).expect("index")]
        }
    }

    fn is_owned_token(token: &str, known: &HashSet<String>, desired: &[String]) -> bool {
        token_slug(token).is_some_and(|slug| known.contains(&slug) || desired.contains(&slug))
    }

    #[test]
    fn property_idempotent_and_preserves_foreign_text() {
        let pieces = [
            "8A",
            "-",
            "Energy",
            "6",
            "#warmup",
            "#WARMUP",
            "#dubs",
            "#dubs/deep",
            "#dub",
            "#other",
            "🔥",
            "/*",
            "House",
            "/",
            "*/",
            "#",
            "x#dubs",
            "café",
            "#Dubs/Deep",
        ];
        let separators = [" ", "  ", "\t", "\n", " \u{3000}"];
        let slugs = ["warmup", "dubs", "dubs/deep", "dub"];
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for _ in 0..3000 {
            let mut text = String::new();
            let n = rng.next() % 8;
            if rng.next().is_multiple_of(4) {
                text.push_str(rng.pick(&separators));
            }
            for i in 0..n {
                if i > 0 {
                    text.push_str(rng.pick(&separators));
                }
                text.push_str(rng.pick(&pieces));
            }
            if rng.next().is_multiple_of(4) {
                text.push_str(rng.pick(&separators));
            }
            let known_set: HashSet<String> = slugs
                .iter()
                .filter(|_| !rng.next().is_multiple_of(3))
                .map(|s| (*s).to_string())
                .collect();
            let desired: Vec<VibePath> = slugs
                .iter()
                .filter(|_| rng.next().is_multiple_of(2))
                .map(|s| VibePath::parse(s).expect("vibe"))
                .collect();
            let desired_slugs: Vec<String> = desired.iter().map(VibePath::slug).collect();

            let existing = (!text.is_empty()).then_some(text.as_str());
            let once = apply_vibes_to_comment(existing, &known_set, &desired);
            let twice = apply_vibes_to_comment(once.as_deref(), &known_set, &desired);
            assert_eq!(once, twice, "not idempotent for {text:?}");

            let out = once.unwrap_or_default();
            let out_tokens: Vec<&str> = tokens(&out).into_iter().map(|r| &out[r]).collect();
            for slug in &desired_slugs {
                assert!(
                    out_tokens
                        .iter()
                        .any(|t| token_slug(t).as_deref() == Some(slug)),
                    "{slug} missing from {out:?}"
                );
            }
            for token in &out_tokens {
                if let Some(slug) = token_slug(token) {
                    assert!(
                        !known_set.contains(&slug) || desired_slugs.contains(&slug),
                        "undesired {token} left in {out:?}"
                    );
                }
            }
            let foreign = |s: &str| -> Vec<String> {
                tokens(s)
                    .into_iter()
                    .map(|r| s[r].to_string())
                    .filter(|t| !is_owned_token(t, &known_set, &desired_slugs))
                    .collect()
            };
            assert_eq!(
                foreign(&text),
                foreign(&out),
                "foreign text changed: {text:?}"
            );
            if desired_slugs.iter().all(|s| {
                tokens(&text)
                    .iter()
                    .any(|r| token_slug(&text[r.clone()]).as_deref() == Some(s))
            }) && !tokens(&text).iter().any(|r| {
                token_slug(&text[r.clone()])
                    .is_some_and(|s| known_set.contains(&s) && !desired_slugs.contains(&s))
            }) && !text.trim().is_empty()
            {
                assert_eq!(out, text, "no-op changed bytes");
            }
        }
    }
}
