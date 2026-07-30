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
