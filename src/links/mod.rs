//! Bidirectional link graph for the Cerebro knowledge base.
//!
//! Uses [`petgraph::Graph`] to maintain a directed graph where nodes are notes
//! and edges represent wikilinks. Provides efficient forward-link, backlink,
//! and neighbourhood queries via a secondary index HashMap.

use std::collections::HashMap;

use petgraph::graph::{EdgeIndex, NodeIndex};
use petgraph::visit::EdgeRef;
use petgraph::Directed;

/// Node in the link graph — represents a single note.
#[derive(Debug, Clone)]
pub struct NoteNode {
    pub id: String,
    pub title: String,
    pub path: String,
}

/// Edge in the link graph — represents a wikilink from one note to another.
#[derive(Debug, Clone)]
pub struct LinkEdge {
    /// Optional surrounding context text for the link.
    pub context: Option<String>,
    /// 1-indexed line number where the link appears.
    pub line_number: Option<usize>,
}

/// Bidirectional link graph backed by petgraph.
pub struct LinkGraph {
    graph: petgraph::Graph<NoteNode, LinkEdge, Directed>,
    /// Maps note id → NodeIndex for O(1) lookup.
    index: HashMap<String, NodeIndex>,
    /// Tracks which link targets are unresolved (no corresponding note).
    unresolved: HashMap<String, Vec<(String, Option<usize>)>>, // target → [(source_id, line)]
}

impl LinkGraph {
    /// Create an empty link graph.
    pub fn new() -> Self {
        Self {
            graph: petgraph::Graph::new(),
            index: HashMap::new(),
            unresolved: HashMap::new(),
        }
    }

    /// Add a note to the graph. Returns the NodeIndex.
    ///
    /// If a note with the same id already exists, it is updated in-place.
    pub fn add_note(&mut self, id: String, title: String, path: String) -> NodeIndex {
        if let Some(&existing_idx) = self.index.get(&id) {
            let node = &mut self.graph[existing_idx];
            node.title = title;
            node.path = path;
            return existing_idx;
        }

        let idx = self.graph.add_node(NoteNode { id: id.clone(), title, path });

        // Check if this note resolves any previously unresolved links
        if let Some(sources) = self.unresolved.remove(&id) {
            for (source_id, line) in sources {
                if let Some(&source_idx) = self.index.get(&source_id) {
                    self.graph.add_edge(
                        source_idx,
                        idx,
                        LinkEdge {
                            context: None,
                            line_number: line,
                        },
                    );
                }
            }
        }

        self.index.insert(id, idx);
        idx
    }

    /// Remove a note and all its edges from the graph.
    ///
    /// Any edges pointing to this note are removed. Links that become
    /// unresolved because of the removal are tracked.
    pub fn remove_note(&mut self, id: &str) -> bool {
        if let Some(idx) = self.index.remove(id) {
            // Collect incoming edges and their sources before removing
            let incoming: Vec<(String, Option<usize>)> = self
                .graph
                .neighbors_directed(idx, petgraph::Direction::Incoming)
                .map(|src| {
                    let src_id = self.graph[src].id.clone();
                    // Try to find the edge and get its line_number
                    let line = self
                        .graph
                        .edges_connecting(src, idx)
                        .next()
                        .and_then(|e| e.weight().line_number);
                    (src_id, line)
                })
                .collect();

            // Remove the node (also removes all connected edges)
            self.graph.remove_node(idx);

            // Track incoming links as now unresolved
            for (source_id, line) in incoming {
                self.unresolved
                    .entry(id.to_owned())
                    .or_default()
                    .push((source_id, line));
            }

            true
        } else {
            false
        }
    }

    /// Add a directed link from `source_id` to `target_id`.
    ///
    /// If the target note does not yet exist in the graph, the link is
    /// recorded as unresolved and will be connected when the target is added.
    pub fn add_link(
        &mut self,
        source_id: &str,
        target_id: &str,
        context: Option<String>,
        line_number: Option<usize>,
    ) -> bool {
        let source_idx = match self.index.get(source_id) {
            Some(&idx) => idx,
            None => return false,
        };

        // Check for duplicate edge (same source, target, line_number)
        if let Some(&target_idx) = self.index.get(target_id) {
            // Check if this exact edge already exists
            let exists = self
                .graph
                .edges_connecting(source_idx, target_idx)
                .any(|e| e.weight().line_number == line_number);
            if exists {
                return true; // Already present
            }

            self.graph.add_edge(
                source_idx,
                target_idx,
                LinkEdge {
                    context,
                    line_number,
                },
            );
            // Remove from unresolved if present
            if let Some(list) = self.unresolved.get_mut(target_id) {
                list.retain(|(s, l)| s != source_id || l != &line_number);
                if list.is_empty() {
                    self.unresolved.remove(target_id);
                }
            }
            true
        } else {
            // Target doesn't exist yet — record as unresolved
            self.unresolved
                .entry(target_id.to_owned())
                .or_default()
                .push((source_id.to_owned(), line_number));
            true
        }
    }

    /// Get all backlinks for a note (notes that link *to* this note).
    ///
    /// Returns a vec of (source note, edge metadata) pairs.
    pub fn get_backlinks(&self, id: &str) -> Vec<(&NoteNode, &LinkEdge)> {
        let idx = match self.index.get(id) {
            Some(&i) => i,
            None => return Vec::new(),
        };

        let mut results = Vec::new();
        let mut incoming = self.graph.neighbors_directed(idx, petgraph::Direction::Incoming);
        while let Some(src_idx) = incoming.next() {
            // Get the edge between src and target
            if let Some(edge) = self.graph.edges_connecting(src_idx, idx).next() {
                results.push((&self.graph[src_idx], edge.weight()));
            }
        }
        results
    }

    /// Get all forward links from a note (notes that this note links *to*).
    ///
    /// Returns a vec of (target note, edge metadata) pairs.
    pub fn get_forward_links(&self, id: &str) -> Vec<(&NoteNode, &LinkEdge)> {
        let idx = match self.index.get(id) {
            Some(&i) => i,
            None => return Vec::new(),
        };

        let mut results = Vec::new();
        let mut outgoing = self.graph.neighbors_directed(idx, petgraph::Direction::Outgoing);
        while let Some(target_idx) = outgoing.next() {
            if let Some(edge) = self.graph.edges_connecting(idx, target_idx).next() {
                results.push((&self.graph[target_idx], edge.weight()));
            }
        }
        results
    }

    /// Get the subgraph around a note to the given depth.
    ///
    /// Returns all notes within `depth` hops and the edges between them.
    /// At depth 0, returns only the note itself. At depth 1, returns the
    /// note plus all directly linked notes.
    pub fn get_graph_around(&self, id: &str, depth: usize) -> LinkSubgraph {
        let start_idx = match self.index.get(id) {
            Some(&i) => i,
            None => {
                return LinkSubgraph {
                    center: None,
                    nodes: Vec::new(),
                    edges: Vec::new(),
                }
            }
        };

        let mut visited = HashMap::new();
        visited.insert(start_idx, 0);

        // BFS
        let mut queue = vec![(start_idx, 0)];
        let mut i = 0;
        while i < queue.len() {
            let (node_idx, d) = queue[i];
            i += 1;

            if d >= depth {
                continue;
            }

            // Follow both directions
            for neighbor in self
                .graph
                .neighbors_undirected(node_idx)
                .collect::<Vec<_>>()
            {
                if !visited.contains_key(&neighbor) {
                    visited.insert(neighbor, d + 1);
                    queue.push((neighbor, d + 1));
                }
            }
        }

        // Collect nodes
        let nodes: Vec<NoteNode> = visited
            .keys()
            .map(|&idx| self.graph[idx].clone())
            .collect();

        // Collect edges between visited nodes
        let edges: Vec<(NoteNode, NoteNode, LinkEdge)> = visited
            .keys()
            .flat_map(|&src_idx| {
                self.graph
                    .neighbors_directed(src_idx, petgraph::Direction::Outgoing)
                    .filter_map(|target_idx| {
                        if visited.contains_key(&target_idx) {
                            self.graph
                                .edges_connecting(src_idx, target_idx)
                                .next()
                                .map(|e| {
                                    (
                                        self.graph[src_idx].clone(),
                                        self.graph[target_idx].clone(),
                                        e.weight().clone(),
                                    )
                                })
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        LinkSubgraph {
            center: Some(self.graph[start_idx].clone()),
            nodes,
            edges,
        }
    }

    /// Get all link targets that don't have a corresponding note in the graph.
    ///
    /// Returns a map of target_id → vec of (source_id, line_number) pairs.
    pub fn unresolved_links(&self) -> &HashMap<String, Vec<(String, Option<usize>)>> {
        &self.unresolved
    }

    /// Get a note by id.
    pub fn get_note(&self, id: &str) -> Option<&NoteNode> {
        self.index.get(id).map(|&idx| &self.graph[idx])
    }

    /// Total number of notes in the graph.
    pub fn note_count(&self) -> usize {
        self.index.len()
    }

    /// Total number of links (edges) in the graph.
    pub fn link_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// Check if a note exists in the graph.
    pub fn contains_note(&self, id: &str) -> bool {
        self.index.contains_key(id)
    }

    /// Remove all links originating from a specific note.
    pub fn clear_links_from(&mut self, source_id: &str) -> bool {
        let source_idx = match self.index.get(source_id) {
            Some(&idx) => idx,
            None => return false,
        };

        // Collect target ids before removing edges
        let targets: Vec<String> = self
            .graph
            .neighbors_directed(source_idx, petgraph::Direction::Outgoing)
            .map(|t| self.graph[t].id.clone())
            .collect();

        // Remove all outgoing edges
        let edges_to_remove: Vec<EdgeIndex> = self
            .graph
            .edges_directed(source_idx, petgraph::Direction::Outgoing)
            .map(|e| e.id())
            .collect();

        for edge_idx in edges_to_remove {
            self.graph.remove_edge(edge_idx);
        }

        // Re-add targets that no longer have any incoming links to unresolved
        // Actually, we just removed outgoing links from source. The target notes
        // still exist — we don't need to mark them as unresolved.
        // But if there are OTHER unresolved references to targets that existed before,
        // those are already tracked separately.

        !targets.is_empty()
    }
}

impl Default for LinkGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// A subgraph extracted around a central note.
#[derive(Debug, Clone)]
pub struct LinkSubgraph {
    /// The central note (None if the note was not found).
    pub center: Option<NoteNode>,
    /// All notes in the subgraph (including center).
    pub nodes: Vec<NoteNode>,
    /// All edges in the subgraph: (source, target, edge_data).
    pub edges: Vec<(NoteNode, NoteNode, LinkEdge)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_query_notes() {
        let mut graph = LinkGraph::new();
        graph.add_note("a".into(), "Note A".into(), "a.md".into());
        graph.add_note("b".into(), "Note B".into(), "b.md".into());

        assert_eq!(graph.note_count(), 2);
        assert!(graph.contains_note("a"));
        assert_eq!(graph.get_note("a").unwrap().title, "Note A");
    }

    #[test]
    fn add_links_and_query() {
        let mut graph = LinkGraph::new();
        graph.add_note("a".into(), "Note A".into(), "a.md".into());
        graph.add_note("b".into(), "Note B".into(), "b.md".into());

        graph.add_link("a", "b", Some("see also".into()), Some(5));

        let forward = graph.get_forward_links("a");
        assert_eq!(forward.len(), 1);
        assert_eq!(forward[0].0.id, "b");
        assert_eq!(forward[0].1.line_number, Some(5));

        let back = graph.get_backlinks("b");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].0.id, "a");
    }

    #[test]
    fn unresolved_links() {
        let mut graph = LinkGraph::new();
        graph.add_note("a".into(), "Note A".into(), "a.md".into());

        // Link to non-existent note
        graph.add_link("a", "missing", None, Some(10));

        assert!(graph.unresolved_links().contains_key("missing"));
        assert_eq!(graph.get_forward_links("a").len(), 0);

        // Now add the missing note — should auto-resolve
        graph.add_note("missing".into(), "Missing Note".into(), "missing.md".into());

        assert!(!graph.unresolved_links().contains_key("missing"));
        assert_eq!(graph.get_forward_links("a").len(), 1);
        assert_eq!(graph.get_backlinks("missing").len(), 1);
    }

    #[test]
    fn remove_note() {
        let mut graph = LinkGraph::new();
        graph.add_note("a".into(), "A".into(), "a.md".into());
        graph.add_note("b".into(), "B".into(), "b.md".into());
        graph.add_link("a", "b", None, None);

        assert!(graph.remove_note("b"));
        assert_eq!(graph.note_count(), 1);
        assert_eq!(graph.get_forward_links("a").len(), 0);
    }

    #[test]
    fn graph_around() {
        let mut graph = LinkGraph::new();
        graph.add_note("a".into(), "A".into(), "a.md".into());
        graph.add_note("b".into(), "B".into(), "b.md".into());
        graph.add_note("c".into(), "C".into(), "c.md".into());
        graph.add_note("d".into(), "D".into(), "d.md".into());

        graph.add_link("a", "b", None, None);
        graph.add_link("b", "c", None, None);
        graph.add_link("c", "d", None, None);

        let sg = graph.get_graph_around("a", 1);
        assert_eq!(sg.nodes.len(), 2); // a, b
        assert!(sg
            .nodes
            .iter()
            .any(|n| n.id == "a"));
        assert!(sg
            .nodes
            .iter()
            .any(|n| n.id == "b"));

        let sg2 = graph.get_graph_around("a", 2);
        assert_eq!(sg2.nodes.len(), 3); // a, b, c
    }

    #[test]
    fn update_existing_note() {
        let mut graph = LinkGraph::new();
        graph.add_note("a".into(), "Old Title".into(), "a.md".into());
        graph.add_note("a".into(), "New Title".into(), "a.md".into());

        assert_eq!(graph.note_count(), 1);
        assert_eq!(graph.get_note("a").unwrap().title, "New Title");
    }
}
