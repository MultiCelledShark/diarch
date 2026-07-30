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

/// Pick the most specific (deepest / highest numeric within siblings) code as a default primary.
pub fn suggest_primary(codes: &[i32]) -> Option<i32> {
    codes.iter().copied().max()
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
