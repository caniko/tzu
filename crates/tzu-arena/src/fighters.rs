use bevy::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::text::{Justify, TextColor, TextFont, TextLayout};
use tzu_core::VerifierDependency;

use crate::data::{ArenaFighterData, ArenaPlanData};
use crate::theme::archetype_colors;

#[derive(Component, Debug, Clone)]
pub struct Fighter {
    pub candidate_id: String,
    pub sketch_id: String,
    pub summary: String,
    pub archetype: VerifierDependency,
    pub max_hp: f32,
    pub hp: f32,
    pub power: f32,
    pub eliminated: bool,
    pub retained: bool,
}

#[derive(Component)]
pub struct HealthBarFill;

#[derive(Component, Default)]
pub struct EntranceAnimation {
    pub progress: f32,
    pub start_x: f32,
    pub target_x: f32,
    pub y: f32,
    pub done: bool,
}

#[derive(Component)]
pub struct ChampionGlow {
    pub timer: Timer,
}

const FIGHTER_WIDTH: f32 = 48.0;
const FIGHTER_HEIGHT: f32 = 64.0;
const HP_BAR_WIDTH: f32 = 60.0;
const HP_BAR_HEIGHT: f32 = 6.0;

pub fn spawn_fighters(
    mut commands: Commands,
    plan_data: Res<ArenaPlanData>,
    mut textures: ResMut<Assets<Image>>,
) {
    let count = plan_data.candidates.len();
    let spacing = 800.0 / (count as f32 + 1.0);
    let start_x = -400.0 + spacing;

    for (i, fighter_data) in plan_data.candidates.iter().enumerate() {
        let colors = archetype_colors(fighter_data.archetype);
        let texture = create_fighter_texture(fighter_data.archetype, &mut textures);
        let x_pos = start_x + (i as f32) * spacing;

        commands.spawn((
            Sprite {
                image: texture,
                custom_size: Some(Vec2::new(FIGHTER_WIDTH, FIGHTER_HEIGHT)),
                color: colors.body,
                ..default()
            },
            Transform::from_xyz(0.0, 1000.0, 1.0),
            Fighter {
                candidate_id: fighter_data.candidate_id.clone(),
                sketch_id: fighter_data.sketch_id.clone(),
                summary: fighter_data.summary.clone(),
                archetype: fighter_data.archetype,
                max_hp: fighter_data.max_hp,
                hp: fighter_data.max_hp,
                power: fighter_data.power.total,
                eliminated: false,
                retained: plan_data.is_retained(&fighter_data.candidate_id),
            },
            EntranceAnimation {
                progress: 0.0,
                start_x: if i % 2 == 0 { -600.0 } else { 600.0 },
                target_x: x_pos,
                y: -100.0,
                done: false,
            },
        ));
    }
}

pub fn update_entrance(
    time: Res<Time>,
    mut query: Query<(&mut EntranceAnimation, &mut Transform)>,
) {
    for (mut anim, mut transform) in query.iter_mut() {
        if anim.done {
            continue;
        }
        anim.progress += time.delta_secs() / 1.5;
        if anim.progress >= 1.0 {
            anim.progress = 1.0;
            anim.done = true;
        }
        let eased = ease_out_cubic(anim.progress);
        let x = anim.start_x + (anim.target_x - anim.start_x) * eased;
        transform.translation.x = x;
        transform.translation.y = anim.y;
    }
}

pub fn sync_health_bars(
    fighters: Query<&Fighter>,
    mut fills: Query<(&mut Sprite, &ChildOf), With<HealthBarFill>>,
) {
    for (mut sprite, child_of) in fills.iter_mut() {
        if let Ok(fighter) = fighters.get(child_of.parent()) {
            let ratio = (fighter.hp / fighter.max_hp).max(0.0);
            if let Some(ref mut size) = sprite.custom_size {
                size.x = HP_BAR_WIDTH * ratio;
            }
            sprite.color = if ratio > 0.6 {
                Color::srgb(0.0, 0.85, 0.3)
            } else if ratio > 0.3 {
                Color::srgb(1.0, 0.65, 0.0)
            } else {
                Color::srgb(0.88, 0.18, 0.18)
            };
        }
    }
}

pub fn mark_eliminated(
    mut fighters: Query<(&mut Fighter, &mut Sprite, &mut Transform)>,
) {
    for (mut fighter, mut sprite, mut transform) in fighters.iter_mut() {
        if fighter.eliminated && fighter.hp > 0.0 {
            fighter.hp = (fighter.hp - 2.0).max(0.0);
            sprite.color = sprite.color.mix(&Color::srgb(0.3, 0.3, 0.3), 0.05);
            transform.translation.y -= 0.5;
        }
    }
}

fn create_fighter_texture(
    archetype: VerifierDependency,
    images: &mut ResMut<Assets<Image>>,
) -> Handle<Image> {
    let w = FIGHTER_WIDTH as u32;
    let h = FIGHTER_HEIGHT as u32;
    let mut image = Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0u8; (w * h * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );

    let w = w as usize;
    let h = h as usize;
    let data = image.data.as_mut().unwrap();

    for y in 0..h {
        for x in 0..w {
            let px = x as f32;
            let py = y as f32;

            let inside = match archetype {
                VerifierDependency::Static => {
                    let body = py > 20.0 && py < 60.0
                        && px > 8.0 && px < 40.0;
                    let helmet = py > 55.0 && py < 64.0
                        && px > (12.0 + (py - 55.0) * 0.8)
                        && px < (36.0 - (py - 55.0) * 0.8);
                    let shield = py > 10.0 && py < 35.0
                        && px > 0.0 && px < 14.0;
                    body || helmet || shield
                }
                VerifierDependency::Repository => {
                    let body = py > 15.0 && py < 62.0
                        && px > 12.0 && px < 36.0;
                    let hat = py > 58.0 && py < 64.0
                        && px > 8.0 && px < 40.0;
                    let bow = py > 20.0 && py < 50.0
                        && ((px > 0.0 && px < 6.0) || (px > 42.0 && px < 48.0));
                    body || hat || bow
                }
                VerifierDependency::Agent => {
                    let robe = py > 10.0 && py < 60.0
                        && px > 6.0 && px < 42.0
                        && px > (6.0 + (py - 10.0) * 0.3)
                        && px < (42.0 - (py - 10.0) * 0.3);
                    let scroll = py > 30.0 && py < 55.0
                        && px > 0.0 && px < 8.0;
                    let hood = py > 52.0 && py < 64.0
                        && px > 12.0 && px < 36.0;
                    robe || scroll || hood
                }
            };

            if inside {
                let i = (y * w + x) * 4;
                if i + 3 < data.len() {
                    data[i + 3] = 255;
                }
            }
        }
    }

    images.add(image)
}

fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}
