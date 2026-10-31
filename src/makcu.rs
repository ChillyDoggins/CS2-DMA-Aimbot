//! MAKCU USB mouse passthrough — ASCII command protocol at 4 Mbaud.
//!
//! Connection sequence:
//!   1. Open at 115200, send the baud-switch frame, close.
//!   2. Reopen at 4_000_000, wait for the `>>> ` prompt.
//! Commands are ASCII text terminated with \r\n, e.g. `km.move(10,5)\r\n`.

use anyhow::{Result, anyhow};
use serialport::SerialPort;
use std::{
    io::{BufRead, BufReader, Write},
    time::Duration,
};

// Binary frame that tells the device to switch to 4 Mbaud.
const BAUD_FRAME_4M: &[u8] = &[0xDE, 0xAD, 0x05, 0x00, 0xA5, 0x00, 0x09, 0x3D, 0x00];

pub struct Makcu {
    port: Box<dyn SerialPort>,
}

impl Makcu {
    pub fn open(name: &str) -> Result<Self> {
        // Step 1: open at 115200 and send the baud-switch frame.
        {
            let mut p = serialport::new(name, 115_200)
                .timeout(Duration::from_millis(200))
                .open()
                .map_err(|e| anyhow!("open {name} @ 115200: {e}"))?;
            p.write_all(BAUD_FRAME_4M)
                .map_err(|e| anyhow!("baud-switch write: {e}"))?;
            std::thread::sleep(Duration::from_millis(100));
        }

        // Step 2: reopen at 4 Mbaud.
        std::thread::sleep(Duration::from_millis(50));
        let mut port = serialport::new(name, 4_000_000)
            .timeout(Duration::from_millis(500))
            .open()
            .map_err(|e| anyhow!("open {name} @ 4M: {e}"))?;

        let _ = port.clear(serialport::ClearBuffer::All);
        std::thread::sleep(Duration::from_millis(50));

        // Verify the device responds.
        port.write_all(b"km.version()\r\n")
            .map_err(|e| anyhow!("version write: {e}"))?;

        let mut reader = BufReader::new(port);
        let mut reply = String::new();
        for _ in 0..3 {
            reply.clear();
            if reader.read_line(&mut reply).is_err() {
                break;
            }
            if reply.contains("km.MAKCU") || reply.contains(">>>") {
                break;
            }
        }

        let port = reader.into_inner();
        Ok(Self { port })
    }

    pub fn list_ports() -> Vec<String> {
        serialport::available_ports()
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.port_name)
            .collect()
    }

    fn cmd(&mut self, s: &str) -> Result<()> {
        self.port
            .write_all(s.as_bytes())
            .map_err(|e| anyhow!("makcu write: {e}"))
    }

    /// Move mouse relatively. dx/dy are signed HID counts.
    pub fn move_rel(&mut self, dx: i16, dy: i16) -> Result<()> {
        self.cmd(&format!("km.move({dx},{dy})\r\n"))
    }
}
