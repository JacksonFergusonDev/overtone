//! Deciding whether a search result is the same track as a file.
//!
//! Ported from OneTagger's `MatchingUtils`
//! (<https://github.com/Marekkon5/onetagger>, `crates/onetagger-tagger/src/lib.rs`,
//! GPL-3.0, by Marekkon5), which is GPL-3.0 like Overtone. The cleaning steps,
//! the attribute list, exact-then-fuzzy title matching, artist matching, and
//! the defaults (strictness 0.7, 30 s duration tolerance) follow it.
//! Differences: step 2 matches a leading article without the leading space
//! OneTagger's pattern requires (which made it a no-op after trimming), and
//! diacritics are folded with Unicode decomposition instead of `unidecode`.

use std::sync::LazyLock;

use regex::Regex;
use strsim::normalized_levenshtein;
use unicode_normalization::UnicodeNormalization;

/// OneTagger's default strictness: the lowest title or artist similarity
/// (0 to 1) that still counts as a match.
pub const DEFAULT_STRICTNESS: f64 = 0.7;
/// OneTagger's default duration tolerance.
pub const MAX_DURATION_DIFFERENCE_SECS: u64 = 30;

const ATTRIBUTES_TO_REMOVE: [&str; 23] = [
    "(intro)",
    "(clean)",
    "(intro clean)",
    "(dirty)",
    "(intro dirty)",
    "(clean extended)",
    "(intro outro)",
    "(extended)",
    "(instrumental)",
    "(quick hit)",
    "(club version)",
    "(radio version)",
    "(club)",
    "(radio)",
    "(main)",
    "(radio edit)",
    "(ck cut)",
    "(super cut)",
    "(mega cutz)",
    "(snip hitz)",
    "(jd live cut)",
    "(djcity intro)",
    "(vdj jd edit)",
];

static ARTICLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(a|an|the) ").expect("regex"));
static ORIGINAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"((\(|\[)*)original( (mix|version|edit))*((\)|\])*)$").expect("regex")
});
static FEAT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r" (\(|\[)?(feat|ft)\.? .+?(\)|\]|\(|$)").expect("regex"));

/// Step 1: lowercase, dashes to spaces, single spaces.
fn step1(input: &str) -> String {
    let mut out = input.to_lowercase().replace('-', " ");
    while out.contains("  ") {
        out = out.replace("  ", " ");
    }
    out.trim().to_string()
}

/// Step 2: drop a leading a/an/the.
fn step2(input: &str) -> String {
    ARTICLE.replace(input, "").into_owned()
}

/// Step 3: drop a trailing "original mix/version/edit".
fn step3(input: &str) -> String {
    ORIGINAL.replace(input, "").into_owned()
}

/// Step 4: drop DJ-pool attributes such as "(clean)" or "(extended)".
fn step4(input: &str) -> String {
    let mut out = input.to_string();
    for attribute in ATTRIBUTES_TO_REMOVE {
        out = out.replace(attribute, "");
    }
    out
}

/// Step 5: drop "feat." and "ft." credits.
fn step5(input: &str) -> String {
    FEAT.replace(input, "").into_owned()
}

/// Step 6: drop "edit".
fn step6(input: &str) -> String {
    input.replace("edit", "")
}

/// Step 7: drop punctuation and fold accents.
fn step7(input: &str) -> String {
    remove_special(input)
}

const STEPS: [fn(&str) -> String; 7] = [step1, step2, step3, step4, step5, step6, step7];

/// Removes punctuation and folds accented letters to ASCII where possible.
pub fn remove_special(input: &str) -> String {
    const SPECIAL: &str = ".,()[]&_\"'-/\\^";
    let mut out: String = input.chars().filter(|c| !SPECIAL.contains(*c)).collect();
    out = out.replace("  ", " ");
    fold(out.trim())
}

fn fold(input: &str) -> String {
    input
        .nfd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect()
}

/// A title cleaned for searching.
pub fn clean_title(input: &str) -> String {
    let out = step1(input);
    let out = step2(&out);
    let out = step3(&out);
    let out = step4(&out);
    let out = step1(&out);
    let out = step5(&out);
    step1(&out)
}

/// A title cleaned for comparing.
pub fn clean_title_matching(input: &str) -> String {
    step7(&step6(&clean_title(input)))
}

/// An artist cleaned for searching (no "feat." credits).
pub fn clean_artist_searching(input: &str) -> String {
    step5(&step1(input)).trim().to_string()
}

fn clean_artists(input: &[String]) -> Vec<String> {
    let mut out: Vec<String> = input
        .iter()
        .map(|a| remove_special(&a.to_lowercase()).trim().to_string())
        .collect();
    out.sort();
    out
}

/// Splits an artist tag into artists on the usual separators.
pub fn split_artists(input: &str) -> Vec<String> {
    static SEP: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\s*(,|;|&|/|\bx\b|\bvs\.?|\bfeat\.?|\bft\.?|\band\b)\s*").expect("regex")
    });
    SEP.split(input)
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whether at least one artist matches: exactly, by containment (for
/// separator differences), or by fuzzy similarity of the whole list.
pub fn match_artist(a: &[String], b: &[String], strictness: f64) -> bool {
    let clean_a = clean_artists(a);
    let clean_b = clean_artists(b);
    if clean_a.iter().any(|artist| clean_b.contains(artist)) {
        return true;
    }
    let joined_a = clean_a.join(" ");
    if clean_b
        .iter()
        .any(|artist| joined_a.contains(artist.as_str()))
    {
        return true;
    }
    let joined_b = clean_b.join(" ");
    if clean_a
        .iter()
        .any(|artist| joined_b.contains(artist.as_str()))
    {
        return true;
    }
    normalized_levenshtein(&clean_a.join(" "), &clean_b.join(", ")) >= strictness
}

/// Whether two durations are within the tolerance. Unknown durations match.
pub fn match_duration(a_secs: Option<u64>, b_secs: Option<u64>) -> bool {
    match (a_secs, b_secs) {
        (Some(a), Some(b)) if a > 0 && b > 0 => a.abs_diff(b) <= MAX_DURATION_DIFFERENCE_SECS,
        _ => true,
    }
}

/// How well a candidate title matches the file's title: 1.0 for an exact
/// match after some number of cleaning steps, otherwise the fuzzy
/// similarity of the fully cleaned titles, or `None` below `strictness`.
pub fn title_score(file_title: &str, candidate_title: &str, strictness: f64) -> Option<f64> {
    let clean = |steps: usize, input: &str| {
        STEPS[..steps]
            .iter()
            .fold(input.to_string(), |acc, step| step(&acc))
    };
    for steps in 0..STEPS.len() {
        if clean(steps, file_title) == clean(steps, candidate_title) {
            return Some(1.0);
        }
    }
    let score = normalized_levenshtein(
        &clean_title_matching(candidate_title),
        &clean_title_matching(file_title),
    );
    (score >= strictness).then_some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artists(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn cleans_titles_like_onetagger() {
        assert_eq!(clean_title("Hold On (Original Mix)"), "hold on");
        assert_eq!(clean_title("Rhyme Dust (Extended)"), "rhyme dust");
        assert_eq!(clean_title("Paper Planes (feat. Someone)"), "paper planes");
        assert_eq!(clean_title("The Prayer"), "prayer");
        assert_eq!(
            clean_title_matching("Café-Del Mar (Radio Edit)"),
            "cafe del mar"
        );
    }

    #[test]
    fn exact_after_cleaning_scores_one() {
        assert_eq!(
            title_score("Hold On (Original Mix)", "Hold On", 0.7),
            Some(1.0)
        );
        assert_eq!(title_score("Get Dumb", "get dumb", 0.7), Some(1.0));
        let fuzzy = title_score("Wicked & Dark", "Wicked and Dark", 0.7).expect("fuzzy");
        assert!(fuzzy > 0.7 && fuzzy < 1.0);
        assert_eq!(title_score("Hold On", "Let Go", 0.7), None);
    }

    #[test]
    fn matches_artists_across_separators_and_case() {
        assert!(match_artist(
            &artists(&["Sub Focus", "bbyclose"]),
            &artists(&["sub focus"]),
            0.7
        ));
        assert!(match_artist(
            &split_artists("Chase & Status"),
            &artists(&["Chase", "Status"]),
            0.7
        ));
        assert!(!match_artist(
            &artists(&["Koven"]),
            &artists(&["Pendulum"]),
            0.7
        ));
    }

    #[test]
    fn splits_artist_tags() {
        assert_eq!(
            split_artists("Bou, Irah feat. Kanine & Trigga"),
            ["Bou", "Irah", "Kanine", "Trigga"]
        );
        assert_eq!(split_artists("Sammy Virji"), ["Sammy Virji"]);
    }

    #[test]
    fn duration_tolerance() {
        assert!(match_duration(Some(300), Some(320)));
        assert!(!match_duration(Some(300), Some(400)));
        assert!(match_duration(None, Some(400)));
    }
}
