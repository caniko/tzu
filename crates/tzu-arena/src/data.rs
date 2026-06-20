use std::collections::BTreeMap;

use bevy::ecs::resource::Resource;
use tzu_core::{CandidateScore, FrontierDiscardReason, HarnessPlanMetadata, SketchStatus};

use crate::damage::FighterPower;

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
    pub sketch_id: String,
    pub summary: String,
    pub status: SketchStatus,
    pub score: CandidateScore,
    pub power: FighterPower,
    pub max_hp: f32,
    pub archetype: tzu_core::VerifierDependency,
    pub risk: tzu_core::Risk,
    pub cost: tzu_core::CostTier,
    pub verifier_strength: tzu_core::VerifierStrength,
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
                    sketch_id: sketch.id.clone(),
                    summary: sketch.candidate.summary.clone(),
                    status: sketch.status,
                    score: score.clone(),
                    power: FighterPower::from_score(score),
                    max_hp: crate::damage::max_hp_from_score(score),
                    archetype: descriptor.verifier_dependency,
                    risk: descriptor.risk_profile,
                    cost: descriptor.cost_tier,
                    verifier_strength: score.verifier_strength,
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
                            let outcome = crate::damage::compute_fight_outcome(ws, ls);
                            Some(outcome.closeness)
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

    pub fn is_discarded(&self, candidate_id: &str) -> bool {
        self.discard_sequence.iter().any(|d| d.candidate_id == candidate_id)
    }

    pub fn is_selected(&self, candidate_id: &str) -> bool {
        self.selected_candidate_id == candidate_id
    }
}
