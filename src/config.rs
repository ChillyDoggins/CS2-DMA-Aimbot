//! Static configuration set from command-line arguments at startup.

pub struct Config {
    /// Serial port name for the MAKCU device, e.g. "COM3".  Empty = auto-detect.
    pub makcu_port: String,
    /// In-game mouse sensitivity (the CS2 "sensitivity" cvar).
    /// Degrees per mouse count = sensitivity * 0.022 (CS2 default m_yaw).
    pub sensitivity: f32,
    /// Maximum angle (degrees) from crosshair to snap to an enemy head.
    pub snap_fov: f32,
    /// Fraction of remaining angle to send per 1ms tick. 1.0 = instant, 0.5 = 2-tick ease.
    pub smooth: f32,
}

impl Config {
    pub fn new(makcu_port: String) -> Self {
        Self {
            makcu_port,
            sensitivity: 2.4,
            snap_fov: 5.0,
            smooth: 0.25,
        }
    }
}
