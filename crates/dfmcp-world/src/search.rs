#![forbid(unsafe_code)]

//! FrankenSearch Hybrid Attention and Knowledge Retrieval Engine.
//!
//! WP-FRK-02: Provides deterministic BM25 full-text search and attention ranking
//! over fortress thoughts, combat reports, announcements, and runbooks.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{EntityId, EventId};

use crate::model::WorldSnapshot;

/// A ranked search match from the FrankenSearch engine.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchHit {
    pub entity_id: Option<EntityId>,
    pub event_id: Option<EventId>,
    pub title: String,
    pub snippet: String,
    pub score: f64,
    pub matched_terms: Vec<String>,
}

/// Tokenizes text into lowercase alphanumeric keywords.
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && w.len() > 1)
        .map(|w| w.to_lowercase())
        .collect()
}

/// Hybrid BM25 Full-Text and Attention Ranking Search Engine.
#[derive(Clone, Debug, Default)]
pub struct FrankenSearchEngine {
    documents_count: usize,
    total_length: usize,
    doc_lengths: Vec<usize>,
    inverted_index: BTreeMap<String, Vec<(usize, usize)>>, // term -> [(doc_index, frequency)]
    doc_metadata: Vec<(Option<EntityId>, Option<EventId>, String, String)>,
}

impl FrankenSearchEngine {
    #[must_use]
    pub fn new() -> Self {
        Self {
            documents_count: 0,
            total_length: 0,
            doc_lengths: Vec::new(),
            inverted_index: BTreeMap::new(),
            doc_metadata: Vec::new(),
        }
    }

    /// Index a document into the search engine.
    pub fn index_document(
        &mut self,
        entity_id: Option<EntityId>,
        event_id: Option<EventId>,
        title: String,
        body: String,
    ) {
        let doc_idx = self.documents_count;
        let terms = tokenize(&format!("{} {}", title, body));
        let len = terms.len();

        let mut term_freqs: BTreeMap<String, usize> = BTreeMap::new();
        for t in terms {
            *term_freqs.entry(t).or_insert(0) += 1;
        }

        for (term, freq) in term_freqs {
            self.inverted_index
                .entry(term)
                .or_default()
                .push((doc_idx, freq));
        }

        self.doc_lengths.push(len);
        self.total_length = self.total_length.saturating_add(len);
        self.documents_count = self.documents_count.saturating_add(1);
        self.doc_metadata.push((entity_id, event_id, title, body));
    }

    /// Index all entities and events from a world snapshot.
    pub fn index_snapshot(&mut self, snapshot: &WorldSnapshot) {
        for (id, entity) in &snapshot.graph.entities {
            let facts_text: Vec<String> = entity
                .fields
                .iter()
                .map(|(name, fact)| format!("{}: {:?}", name, fact.value))
                .collect();
            let body = format!("{} {}", entity.label, facts_text.join(" "));
            self.index_document(Some(*id), None, format!("Entity {:?}", id), body);
        }

        for event in snapshot.graph.events.values() {
            let fields_text: Vec<String> = event
                .fields
                .iter()
                .map(|(name, val)| format!("{}: {:?}", name, val))
                .collect();
            let body = format!("{} {}", event.summary, fields_text.join(" "));
            self.index_document(None, Some(event.id), event.summary.clone(), body);
        }
    }

    /// Query the index using deterministic BM25 scoring.
    #[must_use]
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        let query_terms = tokenize(query);
        if query_terms.is_empty() || self.documents_count == 0 {
            return Vec::new();
        }

        let avgdl = (self.total_length as f64) / (self.documents_count as f64);
        let k1 = 1.2;
        let b = 0.75;

        let mut doc_scores: BTreeMap<usize, (f64, BTreeSet<String>)> = BTreeMap::new();

        for term in &query_terms {
            if let Some(postings) = self.inverted_index.get(term) {
                let n_q = postings.len() as f64;
                let idf = ((self.documents_count as f64 - n_q + 0.5) / (n_q + 0.5) + 1.0).ln();

                for &(doc_idx, freq) in postings {
                    let doc_len = self.doc_lengths[doc_idx] as f64;
                    let tf = freq as f64;
                    let numerator = tf * (k1 + 1.0);
                    let denominator = tf + k1 * (1.0 - b + b * (doc_len / avgdl));
                    let term_score = idf * (numerator / denominator);

                    let entry = doc_scores.entry(doc_idx).or_insert((0.0, BTreeSet::new()));
                    entry.0 += term_score;
                    entry.1.insert(term.clone());
                }
            }
        }

        let mut ranked: Vec<(usize, f64, Vec<String>)> = doc_scores
            .into_iter()
            .map(|(doc_idx, (score, terms))| (doc_idx, score, terms.into_iter().collect()))
            .collect();

        // Sort by score desc, then doc_idx asc for deterministic tie-breaking
        ranked.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });

        ranked
            .into_iter()
            .take(limit)
            .map(|(doc_idx, score, matched_terms)| {
                let (entity_id, event_id, ref title, ref body) = self.doc_metadata[doc_idx];
                let snippet = if body.len() > 120 {
                    format!("{}...", &body[..120])
                } else {
                    body.clone()
                };

                SearchHit {
                    entity_id,
                    event_id,
                    title: title.clone(),
                    snippet,
                    score,
                    matched_terms,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bm25_search_ranking() {
        let mut engine = FrankenSearchEngine::new();

        engine.index_document(
            Some(EntityId::new(1)),
            None,
            "Urist McDwarf".to_owned(),
            "Legendary Miner felt angry after seeing a goblin corpse in the cavern".to_owned(),
        );

        engine.index_document(
            Some(EntityId::new(2)),
            None,
            "Zon Armorsmith".to_owned(),
            "Master Armorsmith felt pleasure near a fine waterfall in the dining hall".to_owned(),
        );

        let hits = engine.search("goblin corpse", 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entity_id, Some(EntityId::new(1)));
        assert!(hits[0].matched_terms.contains(&"goblin".to_owned()));
    }
}
