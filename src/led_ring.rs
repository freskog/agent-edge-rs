use i2cdev::core::I2CDevice;
use i2cdev::linux::LinuxI2CDevice;
use serde::{Deserialize, Serialize};
use std::io;

pub const NUM_LEDS: usize = 12;

const GPO_SERVICER_RESID: u8 = 20;
const GPO_SERVICER_RESID_LED_RING_VALUE: u8 = 18;

const PROBE_ADDRESSES: &[u16] = &[0x2C, 0x28, 0x2A, 0x42, 0x44, 0x50, 0x51];

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl RgbColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub const BLACK: Self = Self::new(0, 0, 0);

    pub fn scaled(self, factor: f32) -> Self {
        Self {
            r: (self.r as f32 * factor).min(255.0) as u8,
            g: (self.g as f32 * factor).min(255.0) as u8,
            b: (self.b as f32 * factor).min(255.0) as u8,
        }
    }

    pub fn from_hsv(h: f32, s: f32, v: f32) -> Self {
        let h = h % 360.0;
        let c = v * s;
        let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
        let m = v - c;

        let (r, g, b) = if h < 60.0 {
            (c, x, 0.0)
        } else if h < 120.0 {
            (x, c, 0.0)
        } else if h < 180.0 {
            (0.0, c, x)
        } else if h < 240.0 {
            (0.0, x, c)
        } else if h < 300.0 {
            (x, 0.0, c)
        } else {
            (c, 0.0, x)
        };

        Self {
            r: ((r + m) * 255.0) as u8,
            g: ((g + m) * 255.0) as u8,
            b: ((b + m) * 255.0) as u8,
        }
    }
}

pub struct LedRing {
    dev: LinuxI2CDevice,
}

impl LedRing {
    /// Open the I2C bus and probe for the ReSpeaker XVF3800 at known addresses.
    pub fn new(i2c_bus: &str) -> io::Result<Self> {
        for &addr in PROBE_ADDRESSES {
            match LinuxI2CDevice::new(i2c_bus, addr) {
                Ok(mut dev) => {
                    let mut buf = [0u8; 1];
                    if dev.read(&mut buf).is_ok() {
                        log::info!("Found ReSpeaker at I2C address 0x{:02X} on {}", addr, i2c_bus);
                        return Ok(Self { dev });
                    }
                }
                Err(_) => continue,
            }
        }

        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("No ReSpeaker device found on {}", i2c_bus),
        ))
    }

    /// Write 12 LED colors to the ring.
    ///
    /// Wire format: [resid, cmd, len, ...48 bytes BGRX]
    pub fn set_leds(&mut self, colors: &[RgbColor; NUM_LEDS]) -> io::Result<()> {
        let mut payload = [0u8; 3 + 48];
        payload[0] = GPO_SERVICER_RESID;
        payload[1] = GPO_SERVICER_RESID_LED_RING_VALUE;
        payload[2] = 48;

        for (i, color) in colors.iter().enumerate() {
            let offset = 3 + i * 4;
            payload[offset] = color.b;
            payload[offset + 1] = color.g;
            payload[offset + 2] = color.r;
            payload[offset + 3] = 0x00;
        }

        self.dev.write(&payload).map_err(|e| {
            io::Error::new(io::ErrorKind::Other, format!("I2C LED write failed: {}", e))
        })
    }

    pub fn all_off(&mut self) -> io::Result<()> {
        self.set_leds(&[RgbColor::BLACK; NUM_LEDS])
    }

    pub fn all_solid(&mut self, color: RgbColor) -> io::Result<()> {
        self.set_leds(&[color; NUM_LEDS])
    }
}
