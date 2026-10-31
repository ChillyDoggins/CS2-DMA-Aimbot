//! Loads offsets from https://github.com/sezzyaep/CS2-OFFSETS.
//! Fresh JSON is downloaded on startup and cached in `offsets/`; if the
//! download fails the cached copy is used instead.

use crate::{log::Show, step};
use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use std::path::Path;

const RAW_BASE: &str = "https://raw.githubusercontent.com/sezzyaep/CS2-OFFSETS/main";
const CACHE_DIR: &str = "offsets";

pub struct Offsets {
    // client.dll globals
    pub dw_entity_list: u64,
    pub dw_local_player_controller: u64,

    // CCSPlayerController
    pub m_isz_player_name: u64,
    pub m_h_player_pawn: u64,

    // C_BaseEntity
    pub m_i_health: u64,
    pub m_i_team_num: u64,
    pub m_p_game_scene_node: u64,

    // CGameSceneNode
    pub m_vec_abs_origin: u64,

    // C_BasePlayerPawn / C_CSPlayerPawn
    pub m_v_old_origin: u64,
    pub m_ang_eye_angles: u64,

    // Pointer from C_BasePlayerPawn to its movement-services sub-object.
    pub m_p_movement_services: u64,
    // Duck blend amount inside CCSPlayer_MovementServices: 0.0 = standing, 1.0 = crouched.
    pub m_fl_duck_amount: u64,

    // Direct pointer to the local player's C_CSPlayerPawn (faster than walking the entity list).
    pub dw_local_player_pawn: u64,

    // Address of KeyButton.state (u32) for attack1 (left mouse). Bit 0 = currently held.
    pub dw_attack1: u64,
}

fn load_json(name: &str) -> Result<Value> {
    let cache_path = Path::new(CACHE_DIR).join(name);
    let url = format!("{RAW_BASE}/{name}");

    let text = match step!(true, fetch(&url), "download {url}") {
        Ok(Body(text)) => {
            let saved = std::fs::create_dir_all(CACHE_DIR)
                .and_then(|_| std::fs::write(&cache_path, &text))
                .map(|_| cache_path.display().to_string());
            let _ = step!(true, saved.map_err(anyhow::Error::from), "save cache");
            text
        }
        Err(_) => step!(
            true,
            std::fs::read_to_string(&cache_path)
                .with_context(|| format!("no cached {}", cache_path.display())),
            "fall back to cached {}",
            cache_path.display()
        )?,
    };
    step!(
        true,
        serde_json::from_str::<Value>(&text).with_context(|| format!("parsing {name}")),
        "parse {name}"
    )
}

/// A downloaded file; logged as its size rather than the whole text.
struct Body(String);

impl Show for Body {
    fn show(&self) -> String {
        format!("{} bytes", self.0.len())
    }
}

impl Show for Value {
    fn show(&self) -> String {
        "ok".into()
    }
}

fn fetch(url: &str) -> Result<Body> {
    Ok(Body(ureq::get(url).call()?.into_string()?))
}

fn global(offsets: &Value, name: &str) -> Result<u64> {
    step!(
        true,
        offsets["client.dll"][name].as_u64().ok_or_else(|| anyhow!("not found in offsets.json")),
        "  client.dll!{name}"
    )
}

fn field(client: &Value, class: &str, name: &str) -> Result<u64> {
    step!(
        true,
        client["client.dll"]["classes"][class]["fields"][name]["offset"]
            .as_u64()
            .ok_or_else(|| anyhow!("not found in client_dll.json")),
        "  {class}::{name}"
    )
}

impl Offsets {
    pub fn load() -> Result<Self> {
        println!("== Loading offsets ==");
        let off = load_json("offsets.json")?;
        let cl = load_json("client_dll.json")?;
        let btn = load_json("buttons.json")?;
        println!("== Looking up offsets ==");

        Ok(Self {
            dw_entity_list: global(&off, "dwEntityList")?,
            dw_local_player_controller: global(&off, "dwLocalPlayerController")?,

            m_isz_player_name: field(&cl, "CBasePlayerController", "m_iszPlayerName")?,
            m_h_player_pawn: field(&cl, "CCSPlayerController", "m_hPlayerPawn")?,

            m_i_health: field(&cl, "C_BaseEntity", "m_iHealth")?,
            m_i_team_num: field(&cl, "C_BaseEntity", "m_iTeamNum")?,
            m_p_game_scene_node: field(&cl, "C_BaseEntity", "m_pGameSceneNode")?,

            m_vec_abs_origin: field(&cl, "CGameSceneNode", "m_vecAbsOrigin")?,

            m_v_old_origin: field(&cl, "C_BasePlayerPawn", "m_vOldOrigin")?,
            m_ang_eye_angles: field(&cl, "C_CSPlayerPawn", "m_angEyeAngles")?,

            m_p_movement_services: field(&cl, "C_BasePlayerPawn", "m_pMovementServices")?,
            m_fl_duck_amount: field(&cl, "CCSPlayer_MovementServices", "m_flDuckAmount")?,

            dw_local_player_pawn: global(&off, "dwLocalPlayerPawn")?,
            dw_attack1: global(&btn, "attack")?,
        })
    }
}
