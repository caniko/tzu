use tzu_core::{CandidateScore, ObligationBurden, Risk, VerifierStrength};

pub fn fighter_power(score: &CandidateScore) -> f32 {
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

    verifier + risk + obligation + quality + readiness
}

pub fn compute_fight_closeness(winner_score: &CandidateScore, loser_score: &CandidateScore) -> f32 {
    let winner_power = fighter_power(winner_score);
    let loser_power = fighter_power(loser_score);
    if winner_power <= 0.0 {
        return 0.0;
    }
    (loser_power / winner_power).clamp(0.0, 1.0)
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
