use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedNode {
    pub code: i32,
    pub name: String,
    #[serde(default)]
    pub children: Vec<SeedNode>,
}

pub fn flatten(nodes: &[SeedNode], parent: Option<i32>) -> Vec<(i32, String, Option<i32>)> {
    let mut out = Vec::new();
    for n in nodes {
        out.push((n.code, n.name.clone(), parent));
        out.extend(flatten(&n.children, Some(n.code)));
    }
    out
}

/// Prefer a newly suggested primary when current is unset, bare Fiction, or a parent of the suggestion.
pub fn prefer_primary(current: Option<i32>, suggested: Option<i32>) -> Option<i32> {
    match (current, suggested) {
        (c, None) => c,
        (None, s) => s,
        (Some(8000), s) => s,
        (Some(c), Some(s)) if c == s => Some(c),
        (Some(c), Some(s)) if is_more_specific(c, s) => Some(s),
        (c, _) => c,
    }
}

fn is_more_specific(current: i32, suggested: i32) -> bool {
    if current == 8200 && (8201..=8299).contains(&suggested) {
        return true;
    }
    if current == 8400 && (8401..=8499).contains(&suggested) {
        return true;
    }
    if current == 8000 {
        return true;
    }
    false
}

/// Pick the most specific (deepest / highest numeric within siblings) code as a default primary.
pub fn suggest_primary(codes: &[i32]) -> Option<i32> {
    // Prefer leaf fiction/subject codes over bare top-level Fiction (8000)
    codes
        .iter()
        .copied()
        .filter(|c| *c != 8000)
        .max()
        .or_else(|| codes.iter().copied().max())
}

/// Map subject / genre strings (from EPUB or Open Library) onto Diarch taxonomy codes.
/// Returns `(primary, all_codes)`.
pub fn map_subjects_to_codes(subjects: &[String]) -> (Option<i32>, Vec<i32>) {
    let mut codes = Vec::new();
    for raw in subjects {
        if let Some(code) = subject_to_code(raw) {
            if !codes.contains(&code) {
                codes.push(code);
            }
        }
    }
    // Always include parent band when we have a leaf
    let mut with_parents = codes.clone();
    for c in &codes {
        for parent in parent_codes(*c) {
            if !with_parents.contains(&parent) {
                with_parents.push(parent);
            }
        }
    }
    let primary = suggest_primary(&codes).or_else(|| suggest_primary(&with_parents));
    (primary, with_parents)
}

fn parent_codes(code: i32) -> Vec<i32> {
    let mut out = Vec::new();
    if (8201..=8299).contains(&code) {
        out.push(8200);
        out.push(8000);
    } else if (8401..=8499).contains(&code) {
        out.push(8400);
        out.push(8000);
    } else if (8600..=8999).contains(&code) {
        out.push(8000);
    } else if code == 8200 || code == 8400 {
        out.push(8000);
    }
    out
}

fn subject_to_code(subject: &str) -> Option<i32> {
    let s = subject.to_ascii_lowercase();
    let s = s.trim();

    // Avoid classifying "nonfiction" as fiction
    if s.contains("nonfiction") || s.contains("non-fiction") {
        return None;
    }

    // Specific before general
    let rules: &[(&[&str], i32)] = &[
        (&["epic fantasy", "high fantasy"], 8201),
        (&["urban fantasy"], 8203),
        (&["dark fantasy"], 8204),
        (&["grimdark"], 8205),
        (&["cozy fantasy"], 8211),
        (&["romantasy", "romantic fantasy"], 8212),
        (&["sword and sorcery", "sword & sorcery"], 8208),
        (&["space opera"], 8403),
        (&["cyberpunk"], 8405),
        (&["military science fiction", "military sf"], 8404),
        (&["hard science fiction", "hard sf"], 8401),
        (&["dystopia", "dystopian"], 8410),
        (&["post-apocalyptic", "postapocalyptic"], 8415),
        (&["time travel"], 8412),
        (&["alternate history"], 8411),
        (&["young adult", "ya fiction", "juvenile fiction"], 8940),
        (&["new adult"], 8970),
        (&["manga"], 8920),
        (&["comics", "graphic novel"], 8910),
        (&["horror"], 8610),
        (&["mystery"], 8710),
        (&["thriller"], 8730),
        (&["crime"], 8720),
        (&["romance"], 8800),
        (&["historical fiction"], 8860),
        (&["biography"], 9300),
        (&["memoir", "autobiography"], 9200),
        (&["self-help", "self help"], 6100),
        (&["fantasy"], 8200),
        (&["science fiction", "sci-fi", "scifi"], 8400),
        (&["fiction"], 8000),
    ];

    for (keys, code) in rules {
        for k in *keys {
            if s.contains(k) {
                return Some(*code);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatten_preserves_parent_links() {
        let nodes = vec![SeedNode {
            code: 8000,
            name: "Fiction".into(),
            children: vec![SeedNode {
                code: 8200,
                name: "Fantasy".into(),
                children: vec![SeedNode {
                    code: 8201,
                    name: "Epic".into(),
                    children: vec![],
                }],
            }],
        }];
        let flat = flatten(&nodes, None);
        assert_eq!(flat.len(), 3);
        assert_eq!(flat[0], (8000, "Fiction".into(), None));
        assert_eq!(flat[1], (8200, "Fantasy".into(), Some(8000)));
        assert_eq!(flat[2], (8201, "Epic".into(), Some(8200)));
    }

    #[test]
    fn suggest_primary_picks_most_specific_numeric() {
        assert_eq!(suggest_primary(&[8000, 8201, 8940]), Some(8940));
        assert_eq!(suggest_primary(&[]), None);
    }

    #[test]
    fn map_fantasy_subject() {
        let (primary, codes) = map_subjects_to_codes(&["Fantasy".into()]);
        assert_eq!(primary, Some(8200));
        assert!(codes.contains(&8200));
        assert!(codes.contains(&8000));
    }

    #[test]
    fn map_space_opera_subject() {
        let (primary, codes) = map_subjects_to_codes(&["Science Fiction — Space opera".into()]);
        assert_eq!(primary, Some(8403));
        assert!(codes.contains(&8400));
    }

    #[test]
    fn seed_json_parses_and_covers_fiction_bands() {
        let seed = include_str!("../../../taxonomy/seed.json");
        let nodes: Vec<SeedNode> = serde_json::from_str(seed).unwrap();
        let flat = flatten(&nodes, None);
        assert!(flat.len() > 50);
        let codes: std::collections::HashSet<i32> = flat.iter().map(|(c, _, _)| *c).collect();
        assert!(codes.contains(&8201)); // epic fantasy
        assert!(codes.contains(&8403)); // space opera
        assert!(codes.contains(&8920)); // manga
        assert!(codes.contains(&6000));
        assert!(codes.contains(&7000));
        assert!(codes.contains(&9000));
    }
}
