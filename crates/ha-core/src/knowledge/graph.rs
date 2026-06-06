//! Knowledge-base link graph builder (WS1, Phase 2).
//!
//! Pure transforms over the index cache: nodes = notes, edges = **resolved**
//! `[[ ]]` / `![[ ]]` links (broken links contribute nothing). Parallel links
//! between the same pair collapse to one edge; self-loops are dropped (a note
//! linking only to itself shows up as an orphan, consistent with how the graph
//! treats degree-0 nodes). Reused by the owner-plane graph view and the
//! `note_graph` agent tool — the only difference is the node cap.

use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::Result;

use super::db::IndexDb;
use super::types::{GraphEdge, GraphNode, KnowledgeGraph};

/// Build the full link graph for a KB: one node per note, one edge per unique
/// resolved `source → target` pair, with per-node resolved in/out degrees.
pub fn build_kb_graph(db: &IndexDb, kb_id: &str) -> Result<KnowledgeGraph> {
    let notes = db.list_notes(kb_id)?;
    let raw = db.all_resolved_links(kb_id)?;
    let node_ids: HashSet<i64> = notes.iter().map(|n| n.id).collect();

    let mut seen_edges: HashSet<(i64, i64)> = HashSet::new();
    let mut edges: Vec<GraphEdge> = Vec::new();
    let mut in_deg: HashMap<i64, u32> = HashMap::new();
    let mut out_deg: HashMap<i64, u32> = HashMap::new();
    for (src, tgt) in raw {
        // Drop self-loops + edges whose endpoints aren't both real notes.
        if src == tgt || !node_ids.contains(&src) || !node_ids.contains(&tgt) {
            continue;
        }
        // Collapse parallel edges (multiple `[[B]]` in A → one A→B edge).
        if !seen_edges.insert((src, tgt)) {
            continue;
        }
        edges.push(GraphEdge {
            source: src,
            target: tgt,
        });
        *out_deg.entry(src).or_insert(0) += 1;
        *in_deg.entry(tgt).or_insert(0) += 1;
    }

    let nodes = notes
        .into_iter()
        .map(|n| GraphNode {
            in_degree: *in_deg.get(&n.id).unwrap_or(&0),
            out_degree: *out_deg.get(&n.id).unwrap_or(&0),
            id: n.id,
            rel_path: n.rel_path,
            title: n.title,
        })
        .collect();

    Ok(KnowledgeGraph {
        nodes,
        edges,
        truncated: false,
    })
}

/// The ego neighbourhood of `center_id`: every node within `depth` undirected
/// hops, plus the edges between kept nodes. Degrees are kept from the full graph
/// (more informative than the local subgraph degree). An unknown `center_id`
/// yields an empty graph.
pub fn ego_subgraph(graph: &KnowledgeGraph, center_id: i64, depth: usize) -> KnowledgeGraph {
    let mut adj: HashMap<i64, Vec<i64>> = HashMap::new();
    for e in &graph.edges {
        adj.entry(e.source).or_default().push(e.target);
        adj.entry(e.target).or_default().push(e.source);
    }

    let in_graph = graph.nodes.iter().any(|n| n.id == center_id);
    let mut keep: HashSet<i64> = HashSet::new();
    if in_graph {
        keep.insert(center_id);
        let mut visited: HashSet<i64> = HashSet::new();
        visited.insert(center_id);
        let mut frontier: VecDeque<(i64, usize)> = VecDeque::new();
        frontier.push_back((center_id, 0));
        while let Some((node, d)) = frontier.pop_front() {
            if d >= depth {
                continue;
            }
            if let Some(neighbors) = adj.get(&node) {
                for &nb in neighbors {
                    if visited.insert(nb) {
                        keep.insert(nb);
                        frontier.push_back((nb, d + 1));
                    }
                }
            }
        }
    }

    let nodes = graph
        .nodes
        .iter()
        .filter(|n| keep.contains(&n.id))
        .cloned()
        .collect();
    let edges = graph
        .edges
        .iter()
        .filter(|e| keep.contains(&e.source) && keep.contains(&e.target))
        .cloned()
        .collect();
    KnowledgeGraph {
        nodes,
        edges,
        truncated: graph.truncated,
    }
}

/// Cap a graph to its `max_nodes` most-connected nodes (degree desc, id asc for
/// determinism), dropping edges that lose an endpoint and flagging `truncated`.
/// A no-op when already within the cap.
pub fn cap_nodes(graph: KnowledgeGraph, max_nodes: usize) -> KnowledgeGraph {
    if graph.nodes.len() <= max_nodes {
        return graph;
    }
    let mut nodes = graph.nodes;
    nodes.sort_by(|a, b| {
        (b.in_degree + b.out_degree)
            .cmp(&(a.in_degree + a.out_degree))
            .then(a.id.cmp(&b.id))
    });
    nodes.truncate(max_nodes);
    let keep: HashSet<i64> = nodes.iter().map(|n| n.id).collect();
    let edges = graph
        .edges
        .into_iter()
        .filter(|e| keep.contains(&e.source) && keep.contains(&e.target))
        .collect();
    KnowledgeGraph {
        nodes,
        edges,
        truncated: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::{chunker, db::NoteIndexInput, parser};
    use tempfile::tempdir;

    fn input(kb: &str, rel: &str, full: &str) -> NoteIndexInput {
        let parsed = parser::parse_document(full);
        let chunks = chunker::chunk(full, &parsed, &chunker::ChunkConfig::default());
        NoteIndexInput {
            kb_id: kb.into(),
            rel_path: rel.into(),
            title: parsed.title.clone().unwrap_or_else(|| rel.into()),
            frontmatter_json: parsed.frontmatter_json,
            mtime: 1,
            size: full.len() as i64,
            content_hash: crate::knowledge::blake3_hex(full.as_bytes()),
            chunks,
            chunk_embeddings: None,
            embedding_signature: None,
            links: parsed.links,
            tags: parsed.tags,
        }
    }

    fn deg(g: &KnowledgeGraph, rel: &str) -> (u32, u32) {
        let n = g.nodes.iter().find(|n| n.rel_path == rel).unwrap();
        (n.in_degree, n.out_degree)
    }

    #[test]
    fn build_graph_nodes_edges_degrees() {
        let dir = tempdir().unwrap();
        let db = IndexDb::open(&dir.path().join("index.db")).unwrap();
        let kb = "kb1";
        // A → B (twice, collapses), A → C, D is an orphan, E → missing (broken).
        db.replace_note_index(input(
            kb,
            "A.md",
            "# A\n\n[[B]] and again [[B]] plus [[C]].\n",
        ))
        .unwrap();
        db.replace_note_index(input(kb, "B.md", "# B\n\nbody.\n"))
            .unwrap();
        db.replace_note_index(input(kb, "C.md", "# C\n\nbody.\n"))
            .unwrap();
        db.replace_note_index(input(kb, "D.md", "# D\n\njust text.\n"))
            .unwrap();
        db.replace_note_index(input(kb, "E.md", "# E\n\n[[missing]].\n"))
            .unwrap();
        db.reresolve_kb_links(kb).unwrap();

        let g = build_kb_graph(&db, kb).unwrap();
        assert_eq!(g.nodes.len(), 5);
        // Parallel [[B]] collapsed → 2 edges total (A→B, A→C).
        assert_eq!(g.edges.len(), 2);
        assert!(!g.truncated);
        assert_eq!(deg(&g, "A.md"), (0, 2));
        assert_eq!(deg(&g, "B.md"), (1, 0));
        assert_eq!(deg(&g, "C.md"), (1, 0));
        // D (no links) and E (only a broken link) are orphans (degree 0).
        assert_eq!(deg(&g, "D.md"), (0, 0));
        assert_eq!(deg(&g, "E.md"), (0, 0));
    }

    #[test]
    fn ego_subgraph_depth_one() {
        let dir = tempdir().unwrap();
        let db = IndexDb::open(&dir.path().join("index.db")).unwrap();
        let kb = "kb1";
        // chain A → B → C; D unrelated.
        db.replace_note_index(input(kb, "A.md", "# A\n\n[[B]].\n"))
            .unwrap();
        db.replace_note_index(input(kb, "B.md", "# B\n\n[[C]].\n"))
            .unwrap();
        db.replace_note_index(input(kb, "C.md", "# C\n\nbody.\n"))
            .unwrap();
        db.replace_note_index(input(kb, "D.md", "# D\n\nbody.\n"))
            .unwrap();
        db.reresolve_kb_links(kb).unwrap();

        let full = build_kb_graph(&db, kb).unwrap();
        let b = full.nodes.iter().find(|n| n.rel_path == "B.md").unwrap().id;

        // depth 1 around B: A, B, C (not D).
        let ego = ego_subgraph(&full, b, 1);
        let mut paths: Vec<&str> = ego.nodes.iter().map(|n| n.rel_path.as_str()).collect();
        paths.sort();
        assert_eq!(paths, vec!["A.md", "B.md", "C.md"]);
        assert_eq!(ego.edges.len(), 2);
    }

    #[test]
    fn cap_keeps_most_connected() {
        let g = KnowledgeGraph {
            nodes: vec![
                GraphNode {
                    id: 1,
                    rel_path: "hub.md".into(),
                    title: "hub".into(),
                    in_degree: 5,
                    out_degree: 0,
                },
                GraphNode {
                    id: 2,
                    rel_path: "a.md".into(),
                    title: "a".into(),
                    in_degree: 0,
                    out_degree: 1,
                },
                GraphNode {
                    id: 3,
                    rel_path: "b.md".into(),
                    title: "b".into(),
                    in_degree: 0,
                    out_degree: 0,
                },
            ],
            edges: vec![GraphEdge {
                source: 2,
                target: 1,
            }],
            truncated: false,
        };
        let capped = cap_nodes(g, 2);
        assert!(capped.truncated);
        assert_eq!(capped.nodes.len(), 2);
        let ids: HashSet<i64> = capped.nodes.iter().map(|n| n.id).collect();
        assert!(ids.contains(&1) && ids.contains(&2)); // hub + its linker, edge kept
        assert_eq!(capped.edges.len(), 1);
    }
}
