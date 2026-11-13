//! Experimental controller selection. The two waypoint modes use identical,
//! pinned Bot Warfare data. No generated-graph fallback masks missing coverage.
use crate::nav::{NAV_HULL, NAV_SCHEMA, NavEdge, NavGraph, TraversalKind};
use bevy::prelude::Resource;

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BotMode {
    #[default]
    Pocket,
    Waypoints,
    Recompiled,
}
impl BotMode {
    pub fn from_env() -> Self {
        match std::env::var("IW4L_BOT_MODE").as_deref() {
            Ok("waypoints") => Self::Waypoints,
            Ok("recompiled") => Self::Recompiled,
            _ => Self::Pocket,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Pocket => "Pocket",
            Self::Waypoints => "Pocket + BW routes",
            Self::Recompiled => "Recompiled + BW routes",
        }
    }
}

pub fn csv(map: &str) -> Option<&'static str> {
    match map {
        "mp_highrise" => Some(include_str!("../waypoints/mp_highrise_wp.csv")),
        "mp_afghan" => Some(include_str!("../waypoints/mp_afghan_wp.csv")),
        "mp_terminal" => Some(include_str!("../waypoints/mp_terminal_wp.csv")),
        _ => None,
    }
}

pub fn parse(text: &str, digest: u64, generation: u64) -> Result<NavGraph, String> {
    let mut lines = text.lines();
    let expected: usize = lines
        .next()
        .ok_or("missing count")?
        .trim()
        .parse()
        .map_err(|_| "invalid count")?;
    if expected == 0 || expected > u16::MAX as usize {
        return Err("invalid node count".into());
    }
    let mut nodes = Vec::new();
    let mut links = Vec::new();
    let mut types = Vec::new();
    for line in lines.filter(|l| !l.trim().is_empty()) {
        let mut fields = line.split(',');
        let position: Vec<f32> = fields
            .next()
            .ok_or("missing position")?
            .split_whitespace()
            .map(str::parse)
            .collect::<Result<_, _>>()
            .map_err(|_| "invalid position")?;
        if position.len() != 3 || position.iter().any(|f| !f.is_finite()) {
            return Err("invalid position".into());
        }
        nodes.push([position[0], position[1], position[2]]);
        links.push(
            fields
                .next()
                .ok_or("missing links")?
                .split_whitespace()
                .map(str::parse::<usize>)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "invalid link")?,
        );
        types.push(match fields.next().unwrap_or("stand").trim() {
            "stand" | "prone" | "claymore" | "grenade" | "javelin" => TraversalKind::Walk,
            "crouch" => TraversalKind::Crouch,
            "climb" => TraversalKind::Climb,
            other => return Err(format!("unsupported waypoint type: {other}")),
        });
    }
    if nodes.len() != expected {
        return Err("count mismatch".into());
    }
    let mut adj = vec![Vec::new(); nodes.len()];
    for (i, targets) in links.iter().enumerate() {
        for &j in targets {
            if j >= nodes.len() {
                return Err("link out of range".into());
            }
            // Upstream follows the destination node's movement flag.
            let cost = nodes[i]
                .iter()
                .zip(nodes[j])
                .map(|(a, b)| (a - b).powi(2))
                .sum::<f32>()
                .sqrt();
            adj[i].push(NavEdge {
                to: j as u16,
                cost,
                kind: types[j],
            });
        }
    }
    let component = crate::nav::label_components(nodes.len(), &adj);
    Ok(NavGraph {
        generation,
        digest,
        schema: NAV_SCHEMA,
        hull: NAV_HULL,
        nodes,
        adj,
        component,
        ..NavGraph::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_routes_have_valid_directed_links() {
        for map in ["mp_highrise", "mp_afghan", "mp_terminal"] {
            let graph = parse(csv(map).unwrap(), 1, 1).unwrap();
            assert!(graph.nodes.len() > 100);
            assert!(graph.adj.iter().map(Vec::len).sum::<usize>() > graph.nodes.len());
        }
    }
    #[test]
    fn preserves_direction_and_traversal() {
        let g = parse(
            "3\n0 0 0,1,stand,,,\n40 0 0,2,crouch,,,\n80 0 20,,climb,,,\n",
            1,
            1,
        )
        .unwrap();
        assert_eq!(g.adj[0][0].kind, TraversalKind::Crouch);
        assert_eq!(g.adj[1][0].kind, TraversalKind::Climb);
        assert!(g.adj[2].is_empty());
        assert!(parse("1\n0 0 0,4,stand,,,", 1, 1).is_err());
    }
}
