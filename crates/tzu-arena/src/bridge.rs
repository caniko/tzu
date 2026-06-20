use bevy::prelude::*;
use bevy::window::PrimaryWindow;

#[derive(Event)]
pub struct FighterClick {
    pub candidate_id: String,
}

pub fn click_detection(
    windows: Query<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    fighters: Query<(&crate::fighters::Fighter, &Transform, &Sprite)>,
    mut commands: Commands,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let window = match windows.single() {
        Ok(w) => w,
        _ => return,
    };
    let cursor_pos = match window.cursor_position() {
        Some(p) => p,
        _ => return,
    };
    let (camera, camera_transform) = match cameras.single() {
        Ok(c) => c,
        _ => return,
    };
    let world_pos = match camera.viewport_to_world_2d(camera_transform, cursor_pos) {
        Ok(p) => p,
        _ => return,
    };

    for (fighter, transform, sprite) in &fighters {
        let fighter_pos = transform.translation.truncate();
        let sprite_size = sprite.custom_size.map_or(Vec2::splat(48.0), |s| s);
        let half = sprite_size / 2.0;
        let min = fighter_pos - half;
        let max = fighter_pos + half;

        if world_pos.x >= min.x && world_pos.x <= max.x
            && world_pos.y >= min.y && world_pos.y <= max.y
        {
            commands.trigger(FighterClick {
                candidate_id: fighter.candidate_id.clone(),
            });
            break;
        }
    }
}

pub fn dispatch_click_to_js(on: On<FighterClick>) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        use web_sys::CustomEventInit;

        let candidate_id = on.candidate_id.clone();
        let window = web_sys::window().expect("no window");
        let mut init = CustomEventInit::new();
        init.set_detail(&JsValue::from_str(&candidate_id));
        let custom_event = web_sys::CustomEvent::new_with_event_init_dict(
            "tzu-arena:fighter-click",
            &init,
        )
        .expect("cannot create CustomEvent");
        window
            .dispatch_event(&custom_event)
            .expect("cannot dispatch event");
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = on;
}

pub fn dispatch_state_change_to_js(on: On<crate::combat::ArenaStateChanged>) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        use web_sys::CustomEventInit;

        let state = on.state.clone();
        let window = web_sys::window().expect("no window");
        let mut init = CustomEventInit::new();
        init.set_detail(&JsValue::from_str(&state));
        let custom_event = web_sys::CustomEvent::new_with_event_init_dict(
            "tzu-arena:state-change",
            &init,
        )
        .expect("cannot create CustomEvent");
        window
            .dispatch_event(&custom_event)
            .expect("cannot dispatch event");
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = on;
}

pub fn dispatch_complete_to_js(on: On<crate::combat::ArenaComplete>) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsValue;
        use web_sys::CustomEventInit;

        let champion_id = on.champion_id.clone();
        let window = web_sys::window().expect("no window");
        let mut init = CustomEventInit::new();
        init.set_detail(&JsValue::from_str(&champion_id));
        let custom_event = web_sys::CustomEvent::new_with_event_init_dict(
            "tzu-arena:complete",
            &init,
        )
        .expect("cannot create CustomEvent");
        window
            .dispatch_event(&custom_event)
            .expect("cannot dispatch event");
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = on;
}
