use std::sync::Mutex;

use bevy::prelude::*;
use bevy::window::Window;
use wasm_bindgen::prelude::*;

mod arena;
mod bridge;
mod combat;
mod data;
mod damage;
mod fighters;
mod theme;

use arena::{idle_particles, spawn_background, update_particles};
use bridge::{
    click_detection, dispatch_click_to_js, dispatch_complete_to_js, dispatch_state_change_to_js,
};
use combat::{
    battle_tick, capacity_tick, coronation_tick, detect_plan_data, enter_battling,
    enter_capacity_cut, enter_coronation, enter_entrance, enter_validation, entrance_tick,
    fade_quotes, validation_tick, ArenaState,
};
use data::ArenaPlanData;
use fighters::{mark_eliminated, spawn_fighters, sync_health_bars, update_entrance};

static PENDING_DATA: Mutex<Option<String>> = Mutex::new(None);
static SPEED_FACTOR: Mutex<f64> = Mutex::new(1.0);
static SKIP_FLAG: Mutex<bool> = Mutex::new(false);

#[wasm_bindgen]
pub fn set_arena_data(json: &str) {
    if let Ok(mut data) = PENDING_DATA.lock() {
        *data = Some(json.to_string());
    }
}

#[wasm_bindgen]
pub fn set_arena_speed(factor: f64) {
    if let Ok(mut speed) = SPEED_FACTOR.lock() {
        *speed = factor.clamp(0.25, 2.0);
    }
}

#[wasm_bindgen]
pub fn skip_to_result() {
    if let Ok(mut flag) = SKIP_FLAG.lock() {
        *flag = true;
    }
}

fn check_pending_data(mut commands: Commands, plan_data: Option<Res<ArenaPlanData>>) {
    if plan_data.is_some() {
        return;
    }
    let json = match PENDING_DATA.lock() {
        Ok(mut data) => data.take(),
        _ => None,
    };
    if let Some(json_str) = json {
        match serde_json::from_str::<tzu_core::HarnessPlanMetadata>(&json_str) {
            Ok(harness) => {
                let arena_data = ArenaPlanData::from_harness(&harness);
                commands.insert_resource(arena_data);
            }
            Err(e) => {
                bevy::log::error!("Failed to parse arena data: {e}");
            }
        }
    }
}

fn check_skip(mut next_state: ResMut<NextState<ArenaState>>, state: Res<State<ArenaState>>) {
    let should_skip = match SKIP_FLAG.lock() {
        Ok(mut flag) => {
            if *flag {
                *flag = false;
                true
            } else {
                false
            }
        }
        _ => false,
    };
    if should_skip && *state.get() != ArenaState::Waiting {
        next_state.set(ArenaState::Coronation);
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        canvas: Some("#tzu-arena-canvas".into()),
                        fit_canvas_to_parent: true,
                        ..default()
                    }),
                    ..default()
                })
                .set(ImagePlugin::default_nearest()),
        )
        .init_state::<ArenaState>()
        .add_observer(dispatch_click_to_js)
        .add_observer(dispatch_state_change_to_js)
        .add_observer(dispatch_complete_to_js)
        .add_systems(Startup, spawn_background)
        .add_systems(
            Update,
            (
                check_pending_data,
                check_skip,
                detect_plan_data,
                idle_particles,
                update_particles,
            ),
        )
        .add_systems(OnEnter(ArenaState::Entrance), (enter_entrance, spawn_fighters))
        .add_systems(Update, update_entrance.run_if(in_state(ArenaState::Entrance)))
        .add_systems(Update, entrance_tick.run_if(in_state(ArenaState::Entrance)))
        .add_systems(OnEnter(ArenaState::Validation), enter_validation)
        .add_systems(Update, validation_tick.run_if(in_state(ArenaState::Validation)))
        .add_systems(OnEnter(ArenaState::Battling), enter_battling)
        .add_systems(Update, battle_tick.run_if(in_state(ArenaState::Battling)))
        .add_systems(OnEnter(ArenaState::CapacityCut), enter_capacity_cut)
        .add_systems(Update, capacity_tick.run_if(in_state(ArenaState::CapacityCut)))
        .add_systems(OnEnter(ArenaState::Coronation), enter_coronation)
        .add_systems(Update, coronation_tick.run_if(in_state(ArenaState::Coronation)))
        .add_systems(
            Update,
            (
                sync_health_bars,
                mark_eliminated,
                click_detection,
                fade_quotes,
            ),
        )
        .run();
}
