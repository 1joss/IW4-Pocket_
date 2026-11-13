//! V6 offline navigation topology audit. Runs once per graph load, never per bot tick.
//! CSV output is intentionally dependency-free and contains no player identifiers.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{self, Write};
use std::path::Path;

use crate::nav::{NavGraph, TraversalKind};

fn csv(path: &Path, name: &str) -> io::Result<std::io::BufWriter<std::fs::File>> {
    Ok(std::io::BufWriter::new(std::fs::File::create(path.join(name))?))
}

fn undirected(graph: &NavGraph) -> Vec<Vec<usize>> {
    let mut result = vec![Vec::new(); graph.nodes.len()];
    for (from, edges) in graph.adj.iter().enumerate() {
        for e in edges {
            let to = e.to as usize;
            if to >= result.len() || from == to { continue; }
            result[from].push(to);
            result[to].push(from);
        }
    }
    for row in &mut result { row.sort_unstable(); row.dedup(); }
    result
}

// Iterative Kosaraju: no deep recursion on large maps.
fn strong_components(graph: &NavGraph) -> (Vec<usize>, Vec<usize>) {
    let n = graph.nodes.len();
    let mut reverse = vec![Vec::new(); n];
    for (i, row) in graph.adj.iter().enumerate() {
        for edge in row { if let Some(r) = reverse.get_mut(edge.to as usize) { r.push(i); } }
    }
    let mut seen = vec![false; n];
    let mut finish = Vec::with_capacity(n);
    for root in 0..n {
        if seen[root] { continue; }
        seen[root] = true;
        let mut stack = vec![(root, 0usize)];
        while let Some((node, next)) = stack.pop() {
            if next < graph.adj[node].len() {
                stack.push((node, next + 1));
                let other = graph.adj[node][next].to as usize;
                if other < n && !seen[other] { seen[other] = true; stack.push((other, 0)); }
            } else { finish.push(node); }
        }
    }
    let mut labels = vec![usize::MAX; n];
    let mut sizes = Vec::new();
    for root in finish.into_iter().rev() {
        if labels[root] != usize::MAX { continue; }
        let id = sizes.len();
        let mut count = 0;
        let mut stack = vec![root];
        labels[root] = id;
        while let Some(node) = stack.pop() {
            count += 1;
            for &other in &reverse[node] {
                if labels[other] == usize::MAX { labels[other] = id; stack.push(other); }
            }
        }
        sizes.push(count);
    }
    (labels, sizes)
}

// Undirected bridges and articulation points, iterative Tarjan DFS.
// These are *structural* bottlenecks, not estimates of traffic volume.
fn critical_links(adj: &[Vec<usize>]) -> (Vec<(usize, usize)>, Vec<usize>) {
    let n = adj.len();
    let mut tin = vec![usize::MAX; n];
    let mut low = vec![0; n];
    let mut parent = vec![usize::MAX; n];
    let mut children = vec![0usize; n];
    let mut articulation = vec![false; n];
    let mut bridges = Vec::new();
    let mut clock = 0;
    for root in 0..n {
        if tin[root] != usize::MAX { continue; }
        tin[root] = clock; low[root] = clock; clock += 1;
        let mut stack = vec![(root, 0usize)];
        while let Some((v, next)) = stack.pop() {
            if next < adj[v].len() {
                stack.push((v, next + 1));
                let u = adj[v][next];
                if u == parent[v] { continue; }
                if tin[u] == usize::MAX {
                    parent[u] = v; children[v] += 1;
                    tin[u] = clock; low[u] = clock; clock += 1;
                    stack.push((u, 0));
                } else { low[v] = low[v].min(tin[u]); }
            } else if parent[v] != usize::MAX {
                let p = parent[v];
                low[p] = low[p].min(low[v]);
                if low[v] > tin[p] { bridges.push((p.min(v), p.max(v))); }
                if parent[p] != usize::MAX && low[v] >= tin[p] { articulation[p] = true; }
            } else if children[v] > 1 { articulation[v] = true; }
        }
    }
    bridges.sort_unstable();
    (bridges, articulation.iter().enumerate().filter_map(|(i, yes)| yes.then_some(i)).collect())
}

fn reach(graph: &NavGraph, start: usize, blocked: Option<(usize, usize)>) -> Vec<bool> {
    let mut visited = vec![false; graph.nodes.len()];
    if start >= visited.len() { return visited; }
    let mut queue = VecDeque::from([start]); visited[start] = true;
    while let Some(node) = queue.pop_front() {
        for edge in &graph.adj[node] {
            let next = edge.to as usize;
            if next >= visited.len() || blocked == Some((node, next)) || visited[next] { continue; }
            visited[next] = true; queue.push_back(next);
        }
    }
    visited
}

fn nearest(graph: &NavGraph, pos: [f32; 3]) -> Option<(usize, f32)> {
    graph.nodes.iter().enumerate().map(|(i, p)| {
        let d = (p[0]-pos[0]).powi(2) + (p[1]-pos[1]).powi(2) + (p[2]-pos[2]).powi(2);
        (i, d)
    }).min_by(|a,b| a.1.total_cmp(&b.1)).map(|(i,d)| (i,d.sqrt()))
}

/// Exhaustive raw graph dump plus bounded spawn/objective topology diagnostics.
/// `spawns` and `objectives` are authored map positions, not live player positions.
pub fn export(graph: &NavGraph, directory: &Path, spawns: &[[f32; 3]], objectives: &[[f32; 3]]) -> io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let n = graph.nodes.len();
    let undirected = undirected(graph);
    let (scc, scc_sizes) = strong_components(graph);
    let (bridges, articulation) = critical_links(&undirected);
    let bridge_set: BTreeSet<_> = bridges.iter().copied().collect();
    let articulation_set: BTreeSet<_> = articulation.iter().copied().collect();
    let mut incoming = vec![0usize; n];
    let mut kinds = [0usize; 3];
    for row in &graph.adj { for edge in row {
        if let Some(v) = incoming.get_mut(edge.to as usize) { *v += 1; }
        kinds[match edge.kind { TraversalKind::Walk => 0, TraversalKind::Drop => 1, TraversalKind::BreakGlass => 2 }] += 1;
    }}
    let mut nodes = csv(directory, "nav-nodes.csv")?;
    writeln!(nodes, "id,x,y,z,weak_component,strong_component,out_degree,in_degree,undirected_degree,articulation")?;
    for (i,p) in graph.nodes.iter().enumerate() {
        writeln!(nodes,"{i},{},{},{},{},{},{},{},{},{}",p[0],p[1],p[2],graph.component.get(i).copied().unwrap_or(u16::MAX),scc[i],graph.adj[i].len(),incoming[i],undirected[i].len(),u8::from(articulation_set.contains(&i)))?;
    }
    let mut edges = csv(directory, "nav-edges.csv")?;
    writeln!(edges,"from,to,kind,cost,length_xy,delta_z,reverse_present,undirected_bridge")?;
    for (i,row) in graph.adj.iter().enumerate() { for edge in row {
        let j = edge.to as usize;
        if j >= n { continue; }
        let a = graph.nodes[i]; let b = graph.nodes[j];
        let dx = a[0]-b[0]; let dy = a[1]-b[1];
        let reverse = graph.adj[j].iter().any(|r| r.to as usize == i);
        writeln!(edges,"{i},{j},{:?},{},{},{},{},{}",edge.kind,edge.cost,(dx*dx+dy*dy).sqrt(),b[2]-a[2],u8::from(reverse),u8::from(bridge_set.contains(&(i.min(j),i.max(j)))))?;
    }}
    let mut components = csv(directory, "nav-components.csv")?;
    writeln!(components,"kind,id,nodes")?;
    let mut weak = BTreeMap::<u16,usize>::new();
    for &id in &graph.component { *weak.entry(id).or_default() += 1; }
    for (id,size) in weak { writeln!(components,"weak,{id},{size}")?; }
    for (id,size) in scc_sizes.iter().enumerate() { writeln!(components,"strong,{id},{size}")?; }
    let mut critical = csv(directory, "nav-critical.csv")?;
    writeln!(critical,"kind,node_a,node_b,x,y,z")?;
    for (a,b) in bridges { let p=graph.nodes[a]; writeln!(critical,"bridge,{a},{b},{},{},{}",p[0],p[1],p[2])?; }
    for id in articulation { let p=graph.nodes[id]; writeln!(critical,"articulation,{id},,{},{},{}",p[0],p[1],p[2])?; }
    let mut anchors = csv(directory, "nav-anchors.csv")?;
    writeln!(anchors,"kind,index,x,y,z,nearest_node,snap_distance,directed_reachable_nodes,reachable_fraction")?;
    let mut starts = Vec::new();
    for (kind, positions) in [("spawn",spawns),("objective",objectives)] {
        for (index,&pos) in positions.iter().enumerate() {
            if let Some((node,distance)) = nearest(graph,pos) {
                let visited = reach(graph,node,None);
                let count = visited.iter().filter(|&&v|v).count();
                writeln!(anchors,"{kind},{index},{},{},{},{node},{distance},{count},{}",pos[0],pos[1],pos[2],if n==0 {0.0} else {count as f32/n as f32})?;
                if kind == "spawn" { starts.push((index,node,visited)); }
            } else { writeln!(anchors,"{kind},{index},{},{},{},,,0,0",pos[0],pos[1],pos[2])?; }
        }
    }
    // Every authored spawn -> objective pair, with a bounded edge-disjoint-path
    // estimate: remove each edge of one discovered path and test reachability.
    // This distinguishes a single critical corridor from redundant alternatives
    // without expensive all-pairs flow over a 40k-node graph.
    let mut routes = csv(directory, "nav-spawn-objectives.csv")?;
    writeln!(routes,"spawn_index,objective_index,start_node,goal_node,reachable,first_path_hops,first_path_unique_edges,first_path_bridge_edges,has_alternative_for_every_first_path_edge")?;
    for (spawn_index,start,visited) in starts {
        for (objective_index,&pos) in objectives.iter().enumerate() {
            let Some((goal,_)) = nearest(graph,pos) else { continue; };
            if !visited[goal] { writeln!(routes,"{spawn_index},{objective_index},{start},{goal},0,0,0,0,0")?; continue; }
            let mut parent = vec![usize::MAX;n];
            let mut queue=VecDeque::from([start]); parent[start]=start;
            while let Some(v)=queue.pop_front() {
                if v==goal { break; }
                for e in &graph.adj[v] { let u=e.to as usize; if u<n && parent[u]==usize::MAX { parent[u]=v; queue.push_back(u); } }
            }
            let mut path=Vec::new(); let mut at=goal;
            while at!=start && parent[at]!=usize::MAX { path.push((parent[at],at)); at=parent[at]; }
            let bridge_count=path.iter().filter(|&&(a,b)|bridge_set.contains(&(a.min(b),a.max(b)))).count();
            // Avoid O(path_len * graph_size) for long routes. Mark untested with -1.
            let alternative=if path.len()<=16 { i32::from(path.iter().all(|&edge|reach(graph,start,Some(edge))[goal])) } else {-1};
            writeln!(routes,"{spawn_index},{objective_index},{start},{goal},1,{},{},{bridge_count},{alternative}",path.len(),path.len())?;
        }
    }
    let mut summary = csv(directory,"nav-summary.txt")?;
    writeln!(summary,"schema={} hull={} digest={:016x} truncated={} nodes={} edges={} walk={} drop={} glass={} weak_components={} strong_components={} largest_scc={} undirected_bridges={} articulation_nodes={} authored_spawns={} objectives={}",graph.schema,graph.hull,graph.digest,graph.truncated,n,kinds.iter().sum::<usize>(),kinds[0],kinds[1],kinds[2],graph.component.iter().copied().collect::<BTreeSet<_>>().len(),scc_sizes.len(),scc_sizes.iter().copied().max().unwrap_or(0),bridge_set.len(),articulation_set.len(),spawns.len(),objectives.len())?;
    writeln!(summary,"notes: directed edges are authoritative; bridges ignore direction; alternative=1 means each edge of one shortest-hop route has an individually avoidable directed alternative, NOT necessarily two fully edge-disjoint routes; -1 means not tested (route >16 hops).")?;
    Ok(())
}
