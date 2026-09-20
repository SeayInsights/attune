//! Searching AutoEQ's published measurements.
//!
//! AutoEQ maintains an index of every profile it publishes, in a stable and
//! simple format:
//!
//! ```text
//! - [Beyerdynamic DT 990 Pro](./oratory1990/over-ear/Beyerdynamic%20DT%20990%20Pro) by oratory1990
//! - [Beyerdynamic DT 990 Pro (250 Ohm)](./crinacle/GRAS%2043AG-7%20over-ear/...) by crinacle on GRAS 43AG-7
//! ```
//!
//! Parsing that is how Attune offers a search box instead of asking someone to
//! find a file on GitHub and paste it.
//!
//! # Why the measurer and rig are surfaced
//!
//! The same headphone measured on different rigs gives different corrections,
//! and reasonable people disagree about which to use. Presenting only one and
//! calling it "the" curve would hide a real choice. Showing who measured it and
//! on what lets the operator decide, which is the honest presentation of a
//! situation where several answers are defensible.

/// Where the index lives.
pub const INDEX_URL: &str =
    "https://raw.githubusercontent.com/jaakkopasanen/AutoEq/master/results/INDEX.md";

/// Base for building a file URL from an index path.
const RESULTS_BASE: &str = "https://raw.githubusercontent.com/jaakkopasanen/AutoEq/master/results";

/// One published measurement.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Entry {
    /// Display name, e.g. "Beyerdynamic DT 990 Pro (250 Ohm)".
    pub name: String,
    /// Who measured it.
    pub source: String,
    /// The measurement rig, when the index names one.
    pub rig: Option<String>,
    /// Path within `results/`, URL-encoded as the index writes it.
    pub path: String,
}

impl Entry {
    /// A one-line description of provenance.
    pub fn provenance(&self) -> String {
        match &self.rig {
            Some(rig) => format!("{} on {}", self.source, rig),
            None => self.source.clone(),
        }
    }

    /// URL of this entry's parametric EQ export.
    ///
    /// AutoEQ names the file after the directory it sits in, and the index's
    /// path is already URL-encoded, so the last segment is reused verbatim
    /// rather than re-encoding a name that has already been encoded once.
    pub fn parametric_url(&self) -> String {
        let last = self.path.rsplit('/').next().unwrap_or(&self.path);
        format!("{RESULTS_BASE}/{}/{last}%20ParametricEQ.txt", self.path)
    }
}

/// Parse AutoEQ's index.
///
/// Lines that do not match are skipped rather than failing the parse. The index
/// carries a prose header and may gain other content; refusing to read it
/// because of an unfamiliar line would break on a change that does not matter.
pub fn parse_index(text: &str) -> Vec<Entry> {
    let mut entries = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("- [") else {
            continue;
        };
        let Some((name, rest)) = rest.split_once("](") else {
            continue;
        };
        // Split on ") by " rather than the first ')': paths routinely contain
        // parentheses -- "Beyerdynamic DT 990 Pro (250 Ohm)" is one -- and
        // splitting on the first one truncates the path mid-name.
        let Some((path, tail)) = rest.split_once(") by ") else {
            continue;
        };
        let tail = format!("by {tail}");
        let tail = tail.as_str();

        let path = path.trim_start_matches("./").to_string();
        if path.is_empty() {
            continue;
        }

        // " by oratory1990" or " by crinacle on GRAS 43AG-7"
        let tail = tail.trim();
        let attribution = tail.strip_prefix("by ").unwrap_or(tail).trim();
        let (source, rig) = match attribution.split_once(" on ") {
            Some((s, r)) => (s.trim().to_string(), Some(r.trim().to_string())),
            None => (attribution.to_string(), None),
        };

        entries.push(Entry {
            name: name.to_string(),
            source,
            rig,
            path,
        });
    }

    entries
}

/// Search entries by name.
///
/// Every whitespace-separated term must appear, case-insensitively, so "dt 990
/// pro" narrows rather than widening the way a single-substring match would.
pub fn search<'a>(entries: &'a [Entry], query: &str, limit: usize) -> Vec<&'a Entry> {
    let terms: Vec<String> = query.split_whitespace().map(|t| t.to_lowercase()).collect();

    if terms.is_empty() {
        return Vec::new();
    }

    let mut matches: Vec<&Entry> = entries
        .iter()
        .filter(|e| {
            let name = e.name.to_lowercase();
            terms.iter().all(|t| name.contains(t))
        })
        .collect();

    // Shorter names first: an exact model beats a variant of it, which is what
    // someone typing the model name is usually looking for.
    matches.sort_by_key(|e| (e.name.len(), e.name.clone()));
    matches.truncate(limit);
    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# Index
This is a list of all equalization profiles.

- [Beyerdynamic DT 990 Pro](./oratory1990/over-ear/Beyerdynamic%20DT%20990%20Pro) by oratory1990
- [Beyerdynamic DT 990 Pro (250 Ohm)](./crinacle/GRAS%2043AG-7%20over-ear/Beyerdynamic%20DT%20990%20Pro%20(250%20Ohm)) by crinacle on GRAS 43AG-7
- [Beyerdynamic DT 770 Pro](./oratory1990/over-ear/Beyerdynamic%20DT%20770%20Pro) by oratory1990
- [Sennheiser HD 600](./oratory1990/over-ear/Sennheiser%20HD%20600) by oratory1990
";

    #[test]
    fn parses_entries_with_and_without_a_rig() {
        let e = parse_index(SAMPLE);
        assert_eq!(e.len(), 4);

        let oratory = &e[0];
        assert_eq!(oratory.name, "Beyerdynamic DT 990 Pro");
        assert_eq!(oratory.source, "oratory1990");
        assert_eq!(oratory.rig, None);

        let crin = &e[1];
        assert_eq!(crin.source, "crinacle");
        assert_eq!(crin.rig.as_deref(), Some("GRAS 43AG-7"));
        assert_eq!(crin.provenance(), "crinacle on GRAS 43AG-7");
    }

    #[test]
    fn prose_lines_are_skipped_not_treated_as_entries() {
        let e = parse_index(SAMPLE);
        assert!(e.iter().all(|x| !x.name.starts_with('#')));
    }

    #[test]
    fn the_parametric_url_reuses_the_already_encoded_path_segment() {
        let e = parse_index(SAMPLE);
        let url = e[0].parametric_url();
        assert!(
            url.ends_with("Beyerdynamic%20DT%20990%20Pro%20ParametricEQ.txt"),
            "{url}"
        );
        assert!(url.contains("/results/oratory1990/over-ear/"));
        // Re-encoding would produce %2520 and a 404.
        assert!(!url.contains("%2520"), "path was double-encoded: {url}");
    }

    #[test]
    fn every_term_must_match_so_a_query_narrows() {
        let e = parse_index(SAMPLE);

        let broad = search(&e, "beyerdynamic", 10);
        assert_eq!(broad.len(), 3);

        let narrow = search(&e, "dt 990", 10);
        assert_eq!(narrow.len(), 2, "770 must not match a 990 query");

        let narrower = search(&e, "dt 990 250", 10);
        assert_eq!(narrower.len(), 1);
    }

    #[test]
    fn search_is_case_insensitive() {
        let e = parse_index(SAMPLE);
        assert_eq!(search(&e, "DT 990 PRO", 10).len(), 2);
    }

    #[test]
    fn the_plain_model_sorts_above_its_variants() {
        let e = parse_index(SAMPLE);
        let hits = search(&e, "dt 990", 10);
        assert_eq!(hits[0].name, "Beyerdynamic DT 990 Pro");
    }

    #[test]
    fn an_empty_query_returns_nothing_rather_than_everything() {
        let e = parse_index(SAMPLE);
        assert!(search(&e, "   ", 10).is_empty());
    }
}
