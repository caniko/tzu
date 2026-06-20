use bevy::prelude::*;
use bevy::text::{Justify, TextColor, TextFont, TextLayout};
use tzu_core::FrontierDiscardReason;

use crate::data::ArenaPlanData;
use crate::fighters::{ChampionGlow, EntranceAnimation, Fighter};
use crate::theme::SUN_TZU_QUOTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, States, Default)]
pub enum ArenaState {
    #[default]
    Waiting,
    Entrance,
    Validation,
    Battling,
    CapacityCut,
    Coronation,
}

#[derive(Event)]
pub struct ArenaStateChanged {
    pub state: String,
}

#[derive(Event)]
pub struct ArenaComplete {
    pub champion_id: String,
}

#[derive(Resource)]
pub struct StateTimer(pub Timer);

#[derive(Resource)]
pub struct FightQueue {
    pub matches: Vec<QueuedFight>,
    pub current: usize,
}

#[derive(Clone)]
pub struct QueuedFight {
    pub loser_id: String,
    pub closeness: f32,
}

#[derive(Component)]
pub struct DisqualifyFlash {
    pub timer: Timer,
}

#[derive(Component)]
pub struct QuoteOverlay {
    pub timer: Timer,
}

pub fn detect_plan_data(
    plan_data: Option<Res<ArenaPlanData>>,
    mut next_state: ResMut<NextState<ArenaState>>,
    state: Res<State<ArenaState>>,
) {
    if *state.get() != ArenaState::Waiting {
        return;
    }
    if plan_data.is_some() {
        next_state.set(ArenaState::Entrance);
    }
}

pub fn enter_entrance(
    mut commands: Commands,
    plan_data: Res<ArenaPlanData>,
) {
    let quote = get_random_quote();
    commands.spawn((
        Text2d::new(format!("{} {}", quote.0, quote.1)),
        TextFont { font_size: FontSize::Px(14.0), ..default() },
        TextColor(Color::srgba(0.8, 0.8, 0.8, 0.0)),
        TextLayout::justify(Justify::Center),
        Transform::from_xyz(0.0, 200.0, 20.0),
        QuoteOverlay {
            timer: Timer::from_seconds(3.0, TimerMode::Once),
        },
    ));

    commands.trigger(ArenaStateChanged {
        state: "entrance".to_string(),
    });

    let fighter_count = plan_data.candidates.len();
    commands.insert_resource(StateTimer(Timer::from_seconds(
        1.5 + fighter_count as f32 * 0.8,
        TimerMode::Once,
    )));
}

pub fn entrance_tick(
    time: Res<Time>,
    mut timer: ResMut<StateTimer>,
    entrance_query: Query<&EntranceAnimation>,
    mut next_state: ResMut<NextState<ArenaState>>,
) {
    timer.0.tick(time.delta());
    if timer.0.just_finished() {
        let all_done = entrance_query.iter().all(|e| e.done);
        if all_done {
            next_state.set(ArenaState::Validation);
        }
    }
}

pub fn enter_validation(
    mut commands: Commands,
    plan_data: Res<ArenaPlanData>,
    fighter_query: Query<(Entity, &Fighter)>,
) {
    commands.trigger(ArenaStateChanged {
        state: "validation".to_string(),
    });

    for (entity, fighter) in &fighter_query {
        let is_invalid = plan_data
            .discard_sequence
            .iter()
            .any(|d| d.candidate_id == fighter.candidate_id && d.reason == FrontierDiscardReason::Invalid);

        if is_invalid {
            commands.entity(entity).insert(DisqualifyFlash {
                timer: Timer::from_seconds(1.2, TimerMode::Once),
            });
        }
    }

    commands.insert_resource(StateTimer(Timer::from_seconds(2.0, TimerMode::Once)));
}

pub fn validation_tick(
    time: Res<Time>,
    mut timer: ResMut<StateTimer>,
    mut disqualified: Query<(&mut DisqualifyFlash, &mut Fighter, &mut Sprite)>,
    mut next_state: ResMut<NextState<ArenaState>>,
) {
    timer.0.tick(time.delta());

    for (mut flash, mut fighter, mut sprite) in disqualified.iter_mut() {
        flash.timer.tick(time.delta());
        let frac = flash.timer.fraction();
        let blink = (frac * 10.0).fract() > 0.5;
        sprite.color = if blink {
            Color::srgb(1.0, 0.2, 0.2)
        } else {
            Color::srgb(0.3, 0.1, 0.1)
        };
        if flash.timer.just_finished() {
            fighter.eliminated = true;
        }
    }

    if timer.0.just_finished() {
        next_state.set(ArenaState::Battling);
    }
}

pub fn enter_battling(
    mut commands: Commands,
    plan_data: Res<ArenaPlanData>,
) {
    commands.trigger(ArenaStateChanged {
        state: "battling".to_string(),
    });

    let matches: Vec<QueuedFight> = plan_data
        .discard_sequence
        .iter()
        .filter(|d| d.reason == FrontierDiscardReason::Dominated)
        .map(|d| QueuedFight {
            loser_id: d.candidate_id.clone(),
            closeness: d.closeness.unwrap_or(0.5),
        })
        .collect();

    commands.insert_resource(FightQueue {
        matches,
        current: 0,
    });
    commands.insert_resource(StateTimer(Timer::from_seconds(2.5, TimerMode::Once)));
}

pub fn battle_tick(
    time: Res<Time>,
    mut timer: ResMut<StateTimer>,
    mut queue: ResMut<FightQueue>,
    mut fighters: Query<(&mut Fighter, &mut Sprite, &mut Transform)>,
    mut next_state: ResMut<NextState<ArenaState>>,
) {
    if queue.matches.is_empty() || queue.current >= queue.matches.len() {
        next_state.set(ArenaState::CapacityCut);
        return;
    }

    timer.0.tick(time.delta());
    let fight = &queue.matches[queue.current];
    let dmg_this_frame = (60.0 + fight.closeness * 35.0) * time.delta_secs() / 2.5;

    for (mut fighter, _sprite, _transform) in fighters.iter_mut() {
        if fighter.candidate_id == fight.loser_id && !fighter.eliminated {
            fighter.hp = (fighter.hp - dmg_this_frame).max(0.0);
        }
    }

    if timer.0.just_finished() {
        let loser_id = queue.matches[queue.current].loser_id.clone();
        for (mut fighter, _sprite, _transform) in fighters.iter_mut() {
            if fighter.candidate_id == loser_id {
                fighter.hp = 0.0;
                fighter.eliminated = true;
            }
        }
        queue.current += 1;
        timer.0.reset();
    }
}

pub fn enter_capacity_cut(
    mut commands: Commands,
    plan_data: Res<ArenaPlanData>,
    fighter_query: Query<(Entity, &Fighter)>,
) {
    commands.trigger(ArenaStateChanged {
        state: "capacity-cut".to_string(),
    });

    for (entity, fighter) in &fighter_query {
        if !fighter.eliminated && !plan_data.is_retained(&fighter.candidate_id) {
            commands.entity(entity).insert(DisqualifyFlash {
                timer: Timer::from_seconds(0.8, TimerMode::Once),
            });
        }
    }

    commands.insert_resource(StateTimer(Timer::from_seconds(1.5, TimerMode::Once)));
}

pub fn capacity_tick(
    time: Res<Time>,
    mut timer: ResMut<StateTimer>,
    mut fighters: Query<(&mut DisqualifyFlash, &mut Fighter, &mut Sprite)>,
    mut next_state: ResMut<NextState<ArenaState>>,
) {
    timer.0.tick(time.delta());

    for (mut flash, mut fighter, mut sprite) in fighters.iter_mut() {
        flash.timer.tick(time.delta());
        if flash.timer.just_finished() {
            fighter.eliminated = true;
        }
        if fighter.eliminated {
            sprite.color = Color::srgba(0.3, 0.3, 0.3, 0.3);
        }
    }

    if timer.0.just_finished() {
        next_state.set(ArenaState::Coronation);
    }
}

pub fn enter_coronation(
    mut commands: Commands,
    plan_data: Res<ArenaPlanData>,
    fighter_query: Query<(Entity, &Fighter)>,
) {
    commands.trigger(ArenaStateChanged {
        state: "coronation".to_string(),
    });

    let quote = get_random_quote();
    commands.spawn((
        Text2d::new(format!("Victory: {}", quote.1)),
        TextFont { font_size: FontSize::Px(16.0), ..default() },
        TextColor(Color::srgb(1.0, 0.84, 0.0)),
        TextLayout::justify(Justify::Center),
        Transform::from_xyz(0.0, 180.0, 20.0),
        QuoteOverlay {
            timer: Timer::from_seconds(4.0, TimerMode::Once),
        },
    ));

    let champion_id = plan_data.selected_candidate_id.clone();
    for (entity, fighter) in &fighter_query {
        if fighter.candidate_id == champion_id {
            commands.entity(entity).insert(ChampionGlow {
                timer: Timer::from_seconds(3.0, TimerMode::Once),
            });
        }
    }

    commands.insert_resource(StateTimer(Timer::from_seconds(4.0, TimerMode::Once)));
}

pub fn coronation_tick(
    time: Res<Time>,
    mut timer: ResMut<StateTimer>,
    mut champions: Query<(&mut ChampionGlow, &mut Sprite, &mut Transform)>,
    plan_data: Res<ArenaPlanData>,
    mut commands: Commands,
) {
    timer.0.tick(time.delta());

    for (mut glow, mut sprite, mut transform) in champions.iter_mut() {
        glow.timer.tick(time.delta());
        let frac = glow.timer.fraction();
        let pulse = 1.0 + (frac * std::f32::consts::TAU).sin() * 0.05;
        transform.scale = Vec3::splat(pulse);
        sprite.color = Color::srgb(1.0, 0.84 + frac * 0.16, 0.0);
    }

    if timer.0.just_finished() {
        commands.trigger(ArenaComplete {
            champion_id: plan_data.selected_candidate_id.clone(),
        });
    }
}

pub fn fade_quotes(
    time: Res<Time>,
    mut commands: Commands,
    mut quotes: Query<(Entity, &mut QuoteOverlay, &mut TextColor)>,
) {
    for (entity, mut overlay, mut text_color) in quotes.iter_mut() {
        overlay.timer.tick(time.delta());
        let frac = overlay.timer.fraction();
        let alpha = if frac > 0.8 {
            (1.0 - (frac - 0.8) / 0.2)
        } else if frac < 0.2 {
            frac / 0.2
        } else {
            1.0
        };
        text_color.0.set_alpha(alpha);
        if overlay.timer.just_finished() {
            commands.entity(entity).despawn();
        }
    }
}

fn get_random_quote() -> &'static (&'static str, &'static str) {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let idx = (seed as usize) % SUN_TZU_QUOTES.len();
    &SUN_TZU_QUOTES[idx]
}
