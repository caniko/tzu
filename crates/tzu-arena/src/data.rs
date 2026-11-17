use std::collections::BTreeMap;

use bevy::ecs::resource::Resource;
use tzu_core::{CandidateScore, FrontierDiscardReason, HarnessPlanMetadata};

use crate::damage::max_hp_from_score;

#[derive(Debug, Clone, Resource)]
pub struct ArenaPlanData {
    pub candidates: Vec<ArenaFighterData>,
    pub discard_sequence: Vec<ArenaDiscard>,
    pub selected_candidate_id: String,
    pub retained_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ArenaFighterData {
    pub candidate_id: String,
    pub max_hp: f32,
    pub archetype: tzu_core::VerifierDependency,
}

#[derive(Debug, Clone)]
pub struct ArenaDiscard {
    pub candidate_id: String,
    pub reason: FrontierDiscardReason,
    pub closeness: Option<f32>,
}

impl ArenaPlanData {
    pub fn from_harness(harness: &HarnessPlanMetadata) -> Self {
        let score_map: BTreeMap<&str, &CandidateScore> = harness
            .candidates
            .iter()
            .map(|sketch| (sketch.id.as_str(), &sketch.score))
            .collect();

        let candidates: Vec<ArenaFighterData> = harness
            .candidates
            .iter()
            .map(|sketch| {
                let score = &sketch.score;
                let descriptor = &sketch.descriptor;
                ArenaFighterData {
                    candidate_id: sketch.id.clone(),
                    max_hp: max_hp_from_score(score),
                    archetype: descriptor.verifier_dependency,
                }
            })
            .collect();

        let retained_ids: Vec<String> = harness.frontier.retained_candidate_ids.clone();

        let mut discard_sequence: Vec<ArenaDiscard> = Vec::new();
        for discard_info in &harness.frontier.discarded_candidates {
            let candidate_id = &discard_info.candidate_id;
            let reason = discard_info.reason;

            let closeness = if reason == FrontierDiscardReason::Dominated {
                if let Some(winner_id) = retained_ids.first() {
                    let loser_score = score_map.get(candidate_id.as_str());
                    let winner_score = score_map.get(winner_id.as_str());
                    match (winner_score, loser_score) {
                        (Some(ws), Some(ls)) => {
                            Some(crate::damage::compute_fight_closeness(ws, ls))
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            } else {
                None
            };

            discard_sequence.push(ArenaDiscard {
                candidate_id: candidate_id.clone(),
                reason,
                closeness,
            });
        }

        let selected_candidate_id = harness.frontier.selected_candidate_id.clone();

        Self {
            candidates,
            discard_sequence,
            selected_candidate_id,
            retained_ids,
        }
    }

    pub fn is_retained(&self, candidate_id: &str) -> bool {
        self.retained_ids.iter().any(|id| id == candidate_id)
    }
}
