mod config;
mod log;
mod makcu;
mod offsets;

use anyhow::{Context, Result, anyhow};
use config::Config;
use makcu::Makcu;
use memprocfs::{FLAG_NOCACHE, Vmm, VmmProcess};
use offsets::Offsets;
use std::{
    path::PathBuf,
    thread,
    time::Duration,
};

/// Size of a CEntityIdentity in the entity list chunks.
const IDENTITY_STRIDES: [u64; 2] = [0x70, 0x78];
const MAX_PLAYERS: u32 = 64;

#[derive(Clone, Copy)]
#[repr(C)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}


struct Player {
    team:     u8,
    alive:    bool,
    pos:      Vec3,
    yaw:      f32,
    pitch:    f32,
    is_local: bool,
    duck:     f32,
}

struct CachedSlot {
    pawn:         u64,
    scene_node:   u64,
    movement_svc: u64,
    is_local:     bool,
    team:         u8,
}


struct Game<'a> {
    proc: VmmProcess<'a>,
    client: u64,
    off: Offsets,
    stride: u64,
    trace: bool,
}

impl<'a> Game<'a> {
    fn read<T>(&self, va: u64) -> Result<T> {
        self.proc.mem_read_as::<T>(va, FLAG_NOCACHE)
    }

    fn read_ptr(&self, va: u64) -> Result<u64> {
        let p: u64 = self.read(va)?;
        if p == 0 { Err(anyhow!("null pointer at {va:#x}")) } else { Ok(p) }
    }

    fn read_string(&self, va: u64, max: usize) -> Result<String> {
        let bytes = self.proc.mem_read_ex(va, max, FLAG_NOCACHE)?;
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        Ok(String::from_utf8_lossy(&bytes[..end]).into_owned())
    }

    fn entity_list(&self) -> Result<u64> {
        step!(
            self.trace,
            self.read_ptr(self.client + self.off.dw_entity_list),
            "read entity list pointer at client.dll+{:#x}",
            self.off.dw_entity_list
        )
    }


    fn identity_by_index(&self, list: u64, index: u32, stride: u64) -> Result<(u64, u64)> {
        let t = self.trace;
        let index = (index & 0x7FFF) as u64;
        let (chunk_no, slot) = (index >> 9, index & 0x1FF);
        let chunk = step!(
            t,
            self.read_ptr(list + 8 * chunk_no + 0x10),
            "    entity {index}: read chunk[{chunk_no}] pointer"
        )?;
        let identity = chunk + stride * slot;
        let ent = step!(
            t,
            self.read_ptr(identity),
            "    entity {index}: read identity[{slot}] at {identity:#x} -> entity"
        )?;
        step!(
            t,
            self.read::<u64>(ent + 0x10).and_then(|p| {
                if p == identity { Ok(p) } else { Err(anyhow!("points to {p:#x}, expected {identity:#x}")) }
            }),
            "    entity {index}: back-pointer entity+0x10 == identity"
        )?;
        Ok((ent, identity))
    }


    fn detect_stride(&mut self) -> Result<()> {
        let list = self.entity_list()?;
        let trace = std::mem::replace(&mut self.trace, false);
        let found = IDENTITY_STRIDES.into_iter().find(|&stride| {
            let hit = (1..=MAX_PLAYERS).find(|&i| self.identity_by_index(list, i, stride).is_ok());
            let hit = hit.ok_or_else(|| anyhow!("no slot in 1..={MAX_PLAYERS} validates"));
            step!(true, hit.map(|i| format!("slot {i} validates")), "  try stride {stride:#x}").is_ok()
        });
        self.trace = trace;
        self.stride = found.ok_or_else(|| anyhow!("could not validate entity list (not in a match yet?)"))?;
        Ok(())
    }

    fn local_eye_angles(&self) -> Result<(f32, f32)> {
        let pawn = self.read_ptr(self.client + self.off.dw_local_player_pawn)?;
        let angles = self.read::<Vec3>(pawn + self.off.m_ang_eye_angles)?;
        Ok((angles.y, angles.x))
    }

    /// Full scan: resolves which entity-list slots hold players and caches
    /// the stable pointers (controller, pawn, scene_node, movement_svc).
    /// Call once per second; result fed to update_players() every 5 ms.
    fn scan_slots(&self) -> Result<Vec<CachedSlot>> {
        let o = &self.off;
        let list  = self.entity_list()?;
        let local = self.read_ptr(self.client + o.dw_local_player_controller).unwrap_or(0);

        let mut slots = Vec::new();
        for i in 1u32..=MAX_PLAYERS {
            let idx      = i as u64;
            let chunk_no = idx >> 9;
            let slot     = idx & 0x1FF;
            let chunk = match self.read_ptr(list + 8 * chunk_no + 0x10) {
                Ok(c) => c, Err(_) => continue,
            };
            let controller = match self.read_ptr(chunk + self.stride * slot) {
                Ok(c) if c != 0 => c, _ => continue,
            };
            let pawn_handle = match self.read::<u32>(controller + o.m_h_player_pawn) {
                Ok(h) if h != u32::MAX => h, _ => continue,
            };
            // Use non-empty name as proof this slot holds a real player.
            match self.read_string(controller + o.m_isz_player_name, 128) {
                Ok(n) if !n.is_empty() => n, _ => continue,
            };
            let is_local = controller == local;

            let pawn_idx      = (pawn_handle as u64) & 0x7FFF;
            let pawn_chunk_no = pawn_idx >> 9;
            let pawn_slot     = pawn_idx & 0x1FF;
            let pawn_chunk = match self.read_ptr(list + 8 * pawn_chunk_no + 0x10) {
                Ok(c) => c, Err(_) => continue,
            };
            let pawn = match self.read_ptr(pawn_chunk + self.stride * pawn_slot) {
                Ok(p) if p != 0 => p, _ => continue,
            };

            let scene_node   = self.read_ptr(pawn + o.m_p_game_scene_node).unwrap_or(0);
            let movement_svc = self.read_ptr(pawn + o.m_p_movement_services).unwrap_or(0);
            let team         = self.read::<u8>(pawn + o.m_i_team_num).unwrap_or(0);
            slots.push(CachedSlot { pawn, scene_node, movement_svc, is_local, team });
        }
        Ok(slots)
    }

    fn update_players(&self, slots: &[CachedSlot]) -> Vec<Player> {
        let o    = &self.off;
        let zero = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
        slots.iter().map(|s| {
            let health = self.read::<i32>(s.pawn + o.m_i_health).unwrap_or(0);
            let pos = if s.scene_node != 0 {
                self.read::<Vec3>(s.scene_node + o.m_vec_abs_origin).unwrap_or(zero)
            } else {
                self.read::<Vec3>(s.pawn + o.m_v_old_origin).unwrap_or(zero)
            };
            let angles = self.read::<Vec3>(s.pawn + o.m_ang_eye_angles).unwrap_or(zero);
            let duck = if s.movement_svc != 0 {
                self.read::<f32>(s.movement_svc + o.m_fl_duck_amount).unwrap_or(0.0).clamp(0.0, 1.0)
            } else { 0.0 };
            Player {
                team: s.team, alive: health > 0,
                pos, yaw: angles.y, pitch: angles.x,
                is_local: s.is_local, duck,
            }
        }).collect()
    }
}

fn head_z(p: &Player) -> f32 {
    p.pos.z + 68.0 - 18.0 * p.duck
}

fn snap_to_head(local: &Player, players: &[Player], sensitivity: f32, fov: f32) -> Option<(i16, i16)> {
    let eye_z = head_z(local);
    let counts_per_deg = 1.0 / (sensitivity * 0.022);
    let fov_sq = fov * fov;

    let best = players
        .iter()
        .filter(|p| !p.is_local && p.alive && (p.team == 2 || p.team == 3) && p.team != local.team)
        .filter_map(|p| {
            let dx = p.pos.x - local.pos.x;
            let dy = p.pos.y - local.pos.y;
            let dz = (p.pos.z + 60.0) - eye_z;
            let horiz = (dx * dx + dy * dy).sqrt();

            let target_yaw = dy.atan2(dx).to_degrees();
            let target_pitch = -(dz.atan2(horiz).to_degrees());

            let mut dyaw = target_yaw - local.yaw;
            while dyaw > 180.0 { dyaw -= 360.0; }
            while dyaw < -180.0 { dyaw += 360.0; }
            let dpitch = target_pitch - local.pitch;

            let dist_sq = dyaw * dyaw + dpitch * dpitch;
            if dist_sq > fov_sq { return None; }
            Some((dist_sq, dyaw, dpitch))
        })
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    best.map(|(_, dyaw, dpitch)| {
        ((-dyaw * counts_per_deg) as i16, (dpitch * counts_per_deg) as i16)
    })
}

fn find_vmm_dll() -> PathBuf {
    let next_to_exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("vmm.dll")));
    match next_to_exe {
        Some(p) if p.exists() => p,
        _ => PathBuf::from("vmm.dll"),
    }
}

fn prompt(label: &str, default: &str) -> String {
    use std::io::Write;
    if default.is_empty() {
        print!("  {label}: ");
    } else {
        print!("  {label} [default: {default}]: ");
    }
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
    let trimmed = line.trim().to_string();
    if trimmed.is_empty() { default.to_string() } else { trimmed }
}

fn main() -> Result<()> {
    let device = "fpga";

    // ── Startup banner ────────────────────────────────────────────────────
    println!();
    println!(" ██████╗███████╗██████╗      █████╗ ██╗███╗   ███╗██████╗  ██████╗ ████████╗");
    println!("██╔════╝██╔════╝╚════██╗    ██╔══██╗██║████╗ ████║██╔══██╗██╔═══██╗╚══██╔══╝");
    println!("██║     ███████╗ █████╔╝    ███████║██║██╔████╔██║██████╔╝██║   ██║   ██║   ");
    println!("██║     ╚════██║██╔═══╝     ██╔══██║██║██║╚██╔╝██║██╔══██╗██║   ██║   ██║   ");
    println!("╚██████╗███████║███████╗    ██║  ██║██║██║ ╚═╝ ██║██████╔╝╚██████╔╝   ██║   ");
    println!(" ╚═════╝╚══════╝╚══════╝    ╚═╝  ╚═╝╚═╝╚═╝     ╚═╝╚═════╝  ╚═════╝   ╚═╝   ");
    println!("                                                       by chillydoggins");
    println!();

    // ── COM port ──────────────────────────────────────────────────────────
    let available_ports = Makcu::list_ports();
    if available_ports.is_empty() {
        println!("  [!] No serial ports detected. MAKCU will run disabled.");
    } else {
        println!("  Available COM ports: {}", available_ports.join(", "));
    }
    let makcu_port = prompt("COM port (e.g. COM3, blank = auto-scan)", "");

    // ── Sensitivity ───────────────────────────────────────────────────────
    let sens_str = prompt("In-game sensitivity", "2.4");
    let sensitivity: f32 = sens_str.parse().unwrap_or(2.4);

    // ── FOV ──────────────────────────────────────────────────────────────
    let fov_str = prompt("Snap FOV (degrees)", "5.0");
    let snap_fov: f32 = fov_str.parse().unwrap_or(5.0);

    println!();
    println!("  sensitivity={sensitivity}  fov={snap_fov}°  port={}",
        if makcu_port.is_empty() { "auto".to_string() } else { makcu_port.clone() });
    println!();

    let off = Offsets::load()?;
    let config = Config { makcu_port, sensitivity, snap_fov, smooth: 0.25 };

    let dll = find_vmm_dll()
        .to_str()
        .map(String::from)
        .context("vmm.dll path is not valid UTF-8")?;
    let vmm = Vmm::new(&dll, &vec!["", "-device", &device, "-waitinitialize"])
        .context("is the DMA device connected?")?;

    let proc = loop {
        match vmm.process_from_name("cs2.exe") {
            Ok(p) => break p,
            Err(_) => {
                println!("Waiting for cs2.exe...");
                thread::sleep(Duration::from_secs(2));
            }
        }
    };
    let client = proc
        .get_module_base("client.dll")
        .context("is the game fully loaded?")?;

    let mut game = Game {
        proc,
        client,
        off,
        stride: IDENTITY_STRIDES[0],
        trace: false,
    };
    while let Err(e) = game.detect_stride() {
        println!("Waiting for match to start: {e:#}");
        thread::sleep(Duration::from_secs(2));
    }

    // Connect MAKCU: use specified port or auto-scan all available ports.
    let mut makcu: Option<Makcu> = None;
    let ports_to_try: Vec<String> = if !config.makcu_port.is_empty() {
        vec![config.makcu_port.clone()]
    } else {
        Makcu::list_ports()
    };
    for port in &ports_to_try {
        match Makcu::open(port) {
            Ok(m) => {
                println!("MAKCU connected on {port}");
                makcu = Some(m);
                break;
            }
            Err(e) => println!("MAKCU failed on {port}: {e:#}"),
        }
    }
    if makcu.is_none() {
        println!("No MAKCU found — mouse moves disabled (snap deltas will still print)");
    }

    println!("Running — press Ctrl+C to stop");

    let mut cached_slots:   Vec<CachedSlot> = Vec::new();
    let mut cached_players: Vec<Player>      = Vec::new();
    let mut last_scan   = std::time::Instant::now() - Duration::from_secs(2);
    let mut last_tick   = std::time::Instant::now();
    let mut last_print  = std::time::Instant::now();

    let mut acc_x: f32 = 0.0;
    let mut acc_y: f32 = 0.0;

    loop {
        // ── Slow scan: 1 Hz — resolve entity list, cache stable pointers ──
        if last_scan.elapsed().as_millis() >= 1000 {
            match game.scan_slots() {
                Ok(slots) => cached_slots = slots,
                Err(e)    => eprintln!("scan_slots ERR: {e:#}"),
            }
            last_scan = std::time::Instant::now();
        }

        // ── Fast tick: 1 ms — 4 DMA reads/player, then immediate snap ────
        if last_tick.elapsed().as_micros() >= 1000 {
            cached_players = game.update_players(&cached_slots);
            last_tick = std::time::Instant::now();

            // ── Aimbot: runs every fast tick (~1 kHz) ─────────────────────
            if !cached_players.is_empty() {
                let attack_val = game.read::<u32>(game.client + game.off.dw_attack1).unwrap_or(0);
                if attack_val & 1 != 0 {
                    if let Some(idx) = cached_players.iter().position(|p| p.is_local) {
                        if let Ok((yaw, pitch)) = game.local_eye_angles() {
                            cached_players[idx].yaw   = yaw;
                            cached_players[idx].pitch = pitch;
                        }
                        let local = &cached_players[idx];
                        if let Some((dx, dy)) = snap_to_head(
                            local, &cached_players,
                            config.sensitivity, config.snap_fov,
                        ) {
                            acc_x += dx as f32 * config.smooth;
                            acc_y += dy as f32 * config.smooth;
                            let send_x = acc_x as i16;
                            let send_y = acc_y as i16;
                            acc_x -= send_x as f32;
                            acc_y -= send_y as f32;
                            if send_x != 0 || send_y != 0 {
                                if let Some(ref mut m) = makcu {
                                    if m.move_rel(send_x, send_y).is_err() {
                                        eprintln!("MAKCU disconnected");
                                        makcu = None;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // ── Periodic status print: 1 Hz ───────────────────────────────────
        if last_print.elapsed().as_millis() >= 1000 {
            last_print = std::time::Instant::now();
            let alive: Vec<&Player> = cached_players.iter().filter(|p| p.alive).collect();
            println!("[CS2] {} players ({} alive) | slots cached: {}",
                cached_players.len(), alive.len(), cached_slots.len());
        }
    }
}
