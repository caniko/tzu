use tzu_core::{CandidateScore, ObligationBurden, Risk, VerifierStrength};

#[derive(Debug, Clone, Copy)]
pub struct FighterPower {
    pub total: f32,
    pub verifier: f32,
    pub risk: f32,
    pub obligation: f32,
    pub quality: f32,
    pub readiness: f32,
}

impl FighterPower {
    pub fn from_score(score: &CandidateScore) -> Self {
        let verifier = match score.verifier_strength {
            VerifierStrength::Weak => 20.0,
            VerifierStrength::Moderate => 40.0,
            VerifierStrength::Strong => 60.0,
        };
        let risk = match score.risk_profile {
            Risk::Low => 20.0,
            Risk::Medium => 15.0,
            Risk::High => 10.0,
        };
        let obligation = match score.obligation_burden {
            ObligationBurden::None => 15.0,
            ObligationBurden::One => 10.0,
            ObligationBurden::Many => 5.0,
        };
        let quality = (score.task_graph_quality as f32) * 0.2;
        let readiness = (score.execution_readiness as f32) * 0.2;

        Self {
            total: verifier + risk + obligation + quality + readiness,
            verifier,
            risk,
            obligation,
            quality,
            readiness,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FightOutcome {
    pub winner_remaining_hp: f32,
    pub loser_remaining_hp: f32,
    pub closeness: f32,
    pub winner_power: f32,
    pub loser_power: f32,
}

pub fn compute_closeness(winner_power: f32, loser_power: f32) -> f32 {
    if winner_power <= 0.0 {
        return 0.0;
    }
    (loser_power / winner_power).clamp(0.0, 1.0)
}

pub fn compute_fight_outcome(winner_score: &CandidateScore, loser_score: &CandidateScore) -> FightOutcome {
    let winner_power = FighterPower::from_score(winner_score).total;
    let loser_power = FighterPower::from_score(loser_score).total;
    let closeness = compute_closeness(winner_power, loser_power);

    let dmg_to_loser = 60.0 + closeness * 35.0;
    let dmg_to_winner = closeness * closeness * 85.0 + 5.0;

    FightOutcome {
        winner_remaining_hp: (100.0 - dmg_to_winner).max(0.0),
        loser_remaining_hp: (100.0 - dmg_to_loser).max(0.0),
        closeness,
        winner_power,
        loser_power,
    }
}

pub fn max_hp_from_score(score: &CandidateScore) -> f32 {
    let base: f32 = 100.0;
    let vs_bonus: f32 = match score.verifier_strength {
        VerifierStrength::Weak => -10.0,
        VerifierStrength::Moderate => 0.0,
        VerifierStrength::Strong => 10.0,
    };
    let risk_bonus: f32 = match score.risk_profile {
        Risk::Low => 10.0,
        Risk::Medium => 0.0,
        Risk::High => -10.0,
    };
    (base + vs_bonus + risk_bonus).max(40.0)
}
