#![forbid(unsafe_code)]

//! Deterministic in-memory attention-search prototype.
//!
//! WP-FRK-02 laboratory: provides a fixed-point lexical relevance ranking over
//! fortress thoughts, combat reports, announcements, and runbooks. It is not an
//! integration with FrankenSearch.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{DfmcpError, EntityId, ErrorCode, EventId, Result};

use crate::model::WorldSnapshot;

const MAX_SEARCH_DOCUMENTS: usize = 65_536;
const MAX_SEARCH_TITLE_BYTES: usize = 256;
const MAX_SEARCH_BODY_BYTES: usize = 65_536;
const MAX_SEARCH_INDEX_BYTES: usize = 64 * 1024 * 1024;
const MAX_SEARCH_TOKENS: usize = 4_000_000;
const MAX_SEARCH_QUERY_BYTES: usize = 4_096;
const MAX_SEARCH_RESULTS: usize = 1_024;

/// A ranked search match from the FrankenSearch engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub entity_id: Option<EntityId>,
    pub event_id: Option<EventId>,
    pub title: String,
    pub snippet: String,
    pub score_micros: u64,
    pub matched_terms: Vec<String>,
}

/// Tokenizes text into lowercase alphanumeric keywords.
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && w.len() > 1)
        .map(|w| w.to_lowercase())
        .collect()
}

/// Fixed-point full-text and attention ranking laboratory.
#[derive(Clone, Debug, Default)]
pub struct FrankenSearchEngine {
    documents_count: usize,
    total_length: usize,
    indexed_bytes: usize,
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
            indexed_bytes: 0,
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
    ) -> Result<()> {
        if self.documents_count >= MAX_SEARCH_DOCUMENTS
            || title.len() > MAX_SEARCH_TITLE_BYTES
            || body.len() > MAX_SEARCH_BODY_BYTES
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "search document exceeds its count, title, or body bound",
            ));
        }
        let document_bytes = title.len().checked_add(body.len()).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "search document byte count overflowed",
            )
        })?;
        let next_indexed_bytes =
            self.indexed_bytes
                .checked_add(document_bytes)
                .ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "search index byte count overflowed",
                    )
                })?;
        if next_indexed_bytes > MAX_SEARCH_INDEX_BYTES {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "search index reached its explicit byte bound",
            ));
        }
        let doc_idx = self.documents_count;
        let mut terms = tokenize(&title);
        terms.extend(tokenize(&body));
        let len = terms.len();
        let next_total_length = self.total_length.checked_add(len).ok_or_else(|| {
            DfmcpError::new(ErrorCode::BudgetExceeded, "search token count overflowed")
        })?;
        if next_total_length > MAX_SEARCH_TOKENS {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "search index reached its explicit token bound",
            ));
        }
        let next_documents_count = self.documents_count.checked_add(1).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "search document count overflowed",
            )
        })?;

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
        self.total_length = next_total_length;
        self.indexed_bytes = next_indexed_bytes;
        self.documents_count = next_documents_count;
        self.doc_metadata.push((entity_id, event_id, title, body));
        Ok(())
    }

    /// Index all entities and events from a world snapshot.
    pub fn index_snapshot(&mut self, snapshot: &WorldSnapshot) -> Result<()> {
        if !snapshot.hash_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "cannot index a snapshot with an invalid canonical state hash",
            ));
        }
        if snapshot
            .graph
            .entities
            .len()
            .checked_add(snapshot.graph.events.len())
            .is_none_or(|count| count > MAX_SEARCH_DOCUMENTS)
            || snapshot.canonical_bytes().len() > MAX_SEARCH_INDEX_BYTES
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "snapshot exceeds the in-memory search indexing bounds or has an invalid hash",
            ));
        }
        let mut replacement = Self::new();
        for (id, entity) in &snapshot.graph.entities {
            let facts_text: Vec<String> = entity
                .fields
                .iter()
                .map(|(name, fact)| format!("{}: {:?}", name, fact.value))
                .collect();
            let body = format!("{} {}", entity.label, facts_text.join(" "));
            replacement.index_document(Some(*id), None, format!("Entity {:?}", id), body)?;
        }

        for event in snapshot.graph.events.values() {
            let fields_text: Vec<String> = event
                .fields
                .iter()
                .map(|(name, val)| format!("{}: {:?}", name, val))
                .collect();
            let body = format!("{} {}", event.summary, fields_text.join(" "));
            replacement.index_document(None, Some(event.id), event.summary.clone(), body)?;
        }
        *self = replacement;
        Ok(())
    }

    /// Remove every indexed document.
    pub fn clear(&mut self) {
        self.documents_count = 0;
        self.total_length = 0;
        self.indexed_bytes = 0;
        self.doc_lengths.clear();
        self.inverted_index.clear();
        self.doc_metadata.clear();
    }

    /// Query the index using deterministic fixed-point lexical scoring.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        if query.len() > MAX_SEARCH_QUERY_BYTES || limit > MAX_SEARCH_RESULTS {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "search query or result limit exceeds its explicit bound",
            ));
        }
        let query_terms = tokenize(query);
        if query_terms.is_empty() || self.documents_count == 0 {
            return Ok(Vec::new());
        }

        const SCALE: u128 = 1_000_000;
        const K1_MICROS: u128 = 1_200_000;
        const ONE_PLUS_K1_MICROS: u128 = 2_200_000;
        const B_MICROS: u128 = 750_000;

        let mut doc_scores: BTreeMap<usize, (u64, BTreeSet<String>)> = BTreeMap::new();

        for term in &query_terms {
            if let Some(postings) = self.inverted_index.get(term) {
                let document_count = self.documents_count as u128;
                let document_frequency = postings.len() as u128;
                let idf_micros =
                    (document_count + 1).saturating_mul(SCALE) / (document_frequency + 1);

                for &(doc_idx, freq) in postings {
                    let Some(document_length) = self.doc_lengths.get(doc_idx).copied() else {
                        continue;
                    };
                    let document_length = document_length as u128;
                    let frequency = freq as u128;
                    let length_ratio_micros = document_length
                        .saturating_mul(document_count)
                        .saturating_mul(SCALE)
                        / (self.total_length.max(1) as u128);
                    let length_normalization_micros = (SCALE - B_MICROS)
                        .saturating_add(B_MICROS.saturating_mul(length_ratio_micros) / SCALE);
                    let denominator_micros = frequency.saturating_mul(SCALE).saturating_add(
                        K1_MICROS.saturating_mul(length_normalization_micros) / SCALE,
                    );
                    let term_frequency_micros = frequency
                        .saturating_mul(ONE_PLUS_K1_MICROS)
                        .saturating_mul(SCALE)
                        / denominator_micros.max(1);
                    let term_score = idf_micros.saturating_mul(term_frequency_micros) / SCALE;
                    let term_score = match u64::try_from(term_score) {
                        Ok(value) => value,
                        Err(_) => u64::MAX,
                    };
                    let entry = doc_scores.entry(doc_idx).or_insert((0, BTreeSet::new()));
                    entry.0 = entry.0.saturating_add(term_score);
                    entry.1.insert(term.clone());
                }
            }
        }

        let mut ranked: Vec<(usize, u64, Vec<String>)> = doc_scores
            .into_iter()
            .map(|(doc_idx, (score, terms))| (doc_idx, score, terms.into_iter().collect()))
            .collect();

        // Sort by score desc, then doc_idx asc for deterministic tie-breaking
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        Ok(ranked
            .into_iter()
            .take(limit)
            .filter_map(|(doc_idx, score, matched_terms)| {
                let (entity_id, event_id, title, body) = self.doc_metadata.get(doc_idx)?;
                let snippet = if body.len() > 120 {
                    let boundary = body
                        .char_indices()
                        .map(|(index, _)| index)
                        .take_while(|index| *index <= 120)
                        .fold(0, |_, index| index);
                    format!("{}...", &body[..boundary])
                } else {
                    body.clone()
                };
                Some(SearchHit {
                    entity_id: *entity_id,
                    event_id: *event_id,
                    title: title.clone(),
                    snippet,
                    score_micros: score,
                    matched_terms,
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bm25_search_ranking() -> Result<()> {
        let mut engine = FrankenSearchEngine::new();

        engine.index_document(
            Some(EntityId::new(1)),
            None,
            "Urist McDwarf".to_owned(),
            "Legendary Miner felt angry after seeing a goblin corpse in the cavern".to_owned(),
        )?;

        engine.index_document(
            Some(EntityId::new(2)),
            None,
            "Zon Armorsmith".to_owned(),
            "Master Armorsmith felt pleasure near a fine waterfall in the dining hall".to_owned(),
        )?;

        let hits = engine.search("goblin corpse", 5)?;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entity_id, Some(EntityId::new(1)));
        assert!(hits[0].matched_terms.contains(&"goblin".to_owned()));
        Ok(())
    }
}
