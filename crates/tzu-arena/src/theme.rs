use bevy::color::Color;
use tzu_core::{CostTier, Risk, VerifierDependency, VerifierStrength};

#[derive(Debug, Clone, Copy)]
pub struct ArchetypeColors {
    pub body: Color,
    pub accent: Color,
    pub hp_bar: Color,
    pub glow: Color,
}

pub const ARCHETYPE_COLORS: &[(VerifierDependency, ArchetypeColors)] = &[
    (
        VerifierDependency::Static,
        ArchetypeColors {
            body: Color::srgb(0.55, 0.27, 0.07),
            accent: Color::srgb(1.0, 0.84, 0.0),
            hp_bar: Color::srgb(0.83, 0.83, 0.0),
            glow: Color::srgb(1.0, 0.84, 0.0),
        },
    ),
    (
        VerifierDependency::Repository,
        ArchetypeColors {
            body: Color::srgb(0.18, 0.49, 0.20),
            accent: Color::srgb(0.0, 0.90, 0.46),
            hp_bar: Color::srgb(0.0, 0.75, 0.38),
            glow: Color::srgb(0.0, 0.90, 0.46),
        },
    ),
    (
        VerifierDependency::Agent,
        ArchetypeColors {
            body: Color::srgb(0.29, 0.08, 0.55),
            accent: Color::srgb(0.88, 0.25, 0.98),
            hp_bar: Color::srgb(0.75, 0.15, 0.85),
            glow: Color::srgb(0.88, 0.25, 0.98),
        },
    ),
];

pub fn archetype_colors(archetype: VerifierDependency) -> ArchetypeColors {
    ARCHETYPE_COLORS
        .iter()
        .find(|(a, _)| *a == archetype)
        .map(|(_, c)| *c)
        .unwrap_or(ArchetypeColors {
            body: Color::srgb(0.5, 0.5, 0.5),
            accent: Color::srgb(1.0, 1.0, 1.0),
            hp_bar: Color::srgb(0.5, 0.5, 0.5),
            glow: Color::srgb(1.0, 1.0, 1.0),
        })
}

pub fn verifier_strength_color(strength: VerifierStrength) -> Color {
    match strength {
        VerifierStrength::Weak => Color::srgb(0.6, 0.6, 0.6),
        VerifierStrength::Moderate => Color::srgb(0.0, 0.7, 1.0),
        VerifierStrength::Strong => Color::srgb(1.0, 0.84, 0.0),
    }
}

pub fn risk_color(risk: Risk) -> Color {
    match risk {
        Risk::Low => Color::srgb(0.0, 0.85, 0.4),
        Risk::Medium => Color::srgb(1.0, 0.65, 0.0),
        Risk::High => Color::srgb(0.88, 0.18, 0.18),
    }
}

pub fn cost_tier_label(cost: CostTier) -> &'static str {
    match cost {
        CostTier::Low => "Low",
        CostTier::Medium => "Medium",
        CostTier::High => "High",
    }
}

pub const SUN_TZU_QUOTES: &[(&str, &str)] = &[
    ("The supreme art of war", "is to subdue the enemy without fighting."),
    ("Appear weak when", "you are strong, and strong when you are weak."),
    ("If you know the enemy and know yourself", "you need not fear the result of a hundred battles."),
    ("In the midst of chaos,", "there is also opportunity."),
    ("The greatest victory", "is that which requires no battle."),
    ("He will win who knows", "when to fight and when not to fight."),
    ("Treat your men as you would", "your own beloved sons."),
    ("All warfare is based", "on deception."),
    ("There is no instance of a nation", "benefiting from prolonged warfare."),
    ("Victory is the main object", "in war."),
    ("To know your enemy,", "you must become your enemy."),
    ("Let your plans be", "dark and impenetrable as night."),
    ("Engage people with what", "they expect; it is what they will see."),
    ("One may know how to conquer", "without being able to do it."),
    ("A leader leads by example,", "not by force."),
    ("When you surround an army,", "leave an outlet to retreat."),
    ("If your enemy is secure at all points,", "be prepared for him."),
    ("To win one hundred victories", "in one hundred battles is not the highest skill."),
    ("Attack him where he is unprepared,", "appear where you are not expected."),
    ("Rouse him, and learn", "the principle of his activity."),
];

pub const ARENA_BG_COLOR: Color = Color::srgb(0.08, 0.08, 0.12);
pub const GROUND_COLOR: Color = Color::srgb(0.15, 0.12, 0.08);
pub const PLATFORM_COLOR: Color = Color::srgb(0.22, 0.17, 0.10);
pub const MOUNTAIN_COLORS: &[Color] = &[
    Color::srgb(0.12, 0.10, 0.18),
    Color::srgb(0.10, 0.08, 0.15),
    Color::srgb(0.08, 0.06, 0.12),
];
