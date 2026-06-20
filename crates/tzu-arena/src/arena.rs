use bevy::prelude::*;
use bevy::sprite::Sprite;

use crate::theme::{GROUND_COLOR, MOUNTAIN_COLORS, PLATFORM_COLOR};

#[derive(Component)]
pub struct ArenaBackground;

#[derive(Component)]
pub struct Mountain {
    pub layer: usize,
}

#[derive(Component)]
pub struct Ground;

#[derive(Component)]
pub struct Particle {
    pub lifetime: Timer,
    pub velocity: Vec2,
}

pub fn spawn_background(mut commands: Commands) {
    commands.spawn(Camera2d);

    for (i, color) in MOUNTAIN_COLORS.iter().enumerate() {
        let layer = i as f32;
        let width = 900.0 + layer * 60.0;
        let height = 120.0 + layer * 20.0;
        let y = 40.0 + layer * 15.0;

        for seg in 0..6 {
            let seg_i = seg as f32;
            let seg_width = width / 6.0 + 20.0;
            let seg_x = -450.0 + seg_i * (width / 5.0) + (seg_i * 15.0).sin() * 30.0;
            let seg_height = height * (0.4 + ((seg_i * 0.8).sin()).abs() * 0.6);

            commands.spawn((
                Sprite {
                    custom_size: Some(Vec2::new(seg_width, seg_height)),
                    color: *color,
                    ..default()
                },
                Transform::from_xyz(seg_x, y - seg_height / 2.0, -10.0 + layer),
                Mountain { layer: i },
            ));
        }
    }

    commands.spawn((
        Sprite {
            custom_size: Some(Vec2::new(900.0, 60.0)),
            color: GROUND_COLOR,
            ..default()
        },
        Transform::from_xyz(0.0, -190.0, -5.0),
        Ground,
    ));

    commands.spawn((
        Sprite {
            custom_size: Some(Vec2::new(700.0, 16.0)),
            color: PLATFORM_COLOR,
            ..default()
        },
        Transform::from_xyz(0.0, -130.0, 0.0),
        Ground,
    ));

    commands.spawn((
        Sprite {
            custom_size: Some(Vec2::new(720.0, 4.0)),
            color: Color::srgb(0.35, 0.25, 0.12),
            ..default()
        },
        Transform::from_xyz(0.0, -122.0, 0.5),
        Ground,
    ));
}

pub fn idle_particles(
    time: Res<Time>,
    mut commands: Commands,
    query: Query<&ArenaBackground>,
) {
    if (time.elapsed_secs() * 2.0).fract() > 0.95 {
        let x = (time.elapsed_secs() * 0.3).sin() * 300.0;
        if query.single().is_ok() {
            commands.spawn((
                Sprite {
                    custom_size: Some(Vec2::new(2.0, 2.0)),
                    color: Color::srgba(0.8, 0.8, 0.8, 0.3),
                    ..default()
                },
                Transform::from_xyz(x, 200.0, -8.0),
                Particle {
                    lifetime: Timer::from_seconds(3.0, TimerMode::Once),
                    velocity: Vec2::new(x * 0.02, -20.0),
                },
            ));
        }
    }
}

pub fn update_particles(
    time: Res<Time>,
    mut commands: Commands,
    mut particles: Query<(Entity, &mut Particle, &mut Transform, &mut Sprite)>,
) {
    for (entity, mut particle, mut transform, mut sprite) in particles.iter_mut() {
        particle.lifetime.tick(time.delta());
        if particle.lifetime.just_finished() {
            commands.entity(entity).despawn();
            continue;
        }
        transform.translation.x += particle.velocity.x * time.delta_secs();
        transform.translation.y += particle.velocity.y * time.delta_secs();
        sprite.color.set_alpha(1.0 - particle.lifetime.fraction());
    }
}
