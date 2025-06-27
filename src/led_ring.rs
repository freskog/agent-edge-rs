#[cfg(feature = "led_ring")]
mod led_ring_impl {
    use rusb::UsbContext;
    use std::time::Duration;
    use thiserror::Error;

    /// ReSpeaker USB device identifiers
    const RESPEAKER_VID: u16 = 0x2886; // SEEED vendor ID
    const RESPEAKER_4MIC_PID: u16 = 0x0018; // ReSpeaker 4-Mic Array product ID

    /// USB Control Transfer constants (matching Python pyusb implementation)
    const CTRL_OUT: u8 = 0x00;
    const CTRL_TYPE_VENDOR: u8 = 0x40;
    const CTRL_RECIPIENT_DEVICE: u8 = 0x00;
    const USB_REQUEST: u8 = 0;
    const USB_VALUE_INDEX: u16 = 0x1C;
    const USB_TIMEOUT: Duration = Duration::from_millis(1000);

    /// LED ring commands
    #[derive(Debug, Clone)]
    pub enum LedCommand {
        /// Trace mode - LEDs change based on VAD and DOA
        Trace,
        /// Set all LEDs to a single color
        Mono { red: u8, green: u8, blue: u8 },
        /// Listen mode - similar to trace but doesn't turn LEDs off
        Listen,
        /// Wait mode
        Wait,
        /// Speak mode  
        Speak,
        /// Spin mode
        Spin,
        /// Custom mode - set each LED individually (12 LEDs)
        Custom { leds: [(u8, u8, u8); 12] },
        /// Set brightness (0-31)
        SetBrightness { brightness: u8 },
        /// Set color palette
        SetColorPalette {
            color1: (u8, u8, u8),
            color2: (u8, u8, u8),
        },
        /// Set center LED (0=off, 1=on, else=depends on VAD)
        SetCenterLed { mode: u8 },
        /// Show volume level (0-12)
        ShowVolume { volume: u8 },
    }

    #[derive(Error, Debug)]
    pub enum LedRingError {
        #[error("Failed to initialize USB context: {0}")]
        UsbInit(rusb::Error),
        #[error("ReSpeaker device not found")]
        DeviceNotFound,
        #[error("Failed to open device: {0}")]
        DeviceOpen(rusb::Error),
        #[error("Failed to send command: {0}")]
        CommandFailed(rusb::Error),
        #[error("Invalid brightness value: {0} (must be 0-31)")]
        InvalidBrightness(u8),
        #[error("Invalid volume value: {0} (must be 0-12)")]
        InvalidVolume(u8),
        #[error("Failed to claim interface: {0}")]
        InterfaceClaim(rusb::Error),
    }

    /// Controller for ReSpeaker LED ring using direct USB control transfers
    pub struct LedRing {
        device_handle: rusb::DeviceHandle<rusb::Context>,
        _context: rusb::Context, // Keep context alive
    }

    impl LedRing {
        /// Create a new LED ring controller
        pub fn new() -> Result<Self, LedRingError> {
            let context = rusb::Context::new().map_err(LedRingError::UsbInit)?;

            // Find the ReSpeaker device
            let devices = context.devices().map_err(LedRingError::UsbInit)?;

            for device in devices.iter() {
                let device_desc = device.device_descriptor().map_err(LedRingError::UsbInit)?;

                if device_desc.vendor_id() == RESPEAKER_VID
                    && device_desc.product_id() == RESPEAKER_4MIC_PID
                {
                    let device_handle = device.open().map_err(LedRingError::DeviceOpen)?;

                    // Try to claim the vendor-specific interface (interface 3 according to lsusb output)
                    // This might fail if the interface is already claimed by the system
                    if let Err(e) = device_handle.claim_interface(3) {
                        log::warn!(
                            "Could not claim interface 3: {}. LED control may still work.",
                            e
                        );
                    }

                    log::info!("ReSpeaker USB 4-Mic Array found and opened successfully");

                    return Ok(LedRing {
                        device_handle,
                        _context: context,
                    });
                }
            }

            Err(LedRingError::DeviceNotFound)
        }

        /// Send a command to the LED ring using USB control transfer
        /// This matches the Python implementation:
        /// ctrl_transfer(CTRL_OUT | CTRL_TYPE_VENDOR | CTRL_RECIPIENT_DEVICE, 0, command, 0x1C, data, TIMEOUT)
        pub fn send_command(&self, command: LedCommand) -> Result<(), LedRingError> {
            let (cmd_value, data) = match command {
                LedCommand::Trace => (0, vec![0]),
                LedCommand::Mono { red, green, blue } => (1, vec![red, green, blue, 0]),
                LedCommand::Listen => (2, vec![0]),
                LedCommand::Wait => (3, vec![0]),
                LedCommand::Speak => (4, vec![0]),
                LedCommand::Spin => (5, vec![0]),
                LedCommand::Custom { leds } => {
                    let mut data = Vec::with_capacity(48); // 12 LEDs * 4 bytes each
                    for (r, g, b) in leds.iter() {
                        data.extend_from_slice(&[*r, *g, *b, 0]);
                    }
                    (6, data)
                }
                LedCommand::SetBrightness { brightness } => {
                    if brightness > 31 {
                        return Err(LedRingError::InvalidBrightness(brightness));
                    }
                    (0x20, vec![brightness])
                }
                LedCommand::SetColorPalette { color1, color2 } => (
                    0x21,
                    vec![
                        color1.0, color1.1, color1.2, 0, color2.0, color2.1, color2.2, 0,
                    ],
                ),
                LedCommand::SetCenterLed { mode } => (0x22, vec![mode]),
                LedCommand::ShowVolume { volume } => {
                    if volume > 12 {
                        return Err(LedRingError::InvalidVolume(volume));
                    }
                    (0x23, vec![volume])
                }
            };

            // Prepare control transfer parameters
            let request_type = CTRL_OUT | CTRL_TYPE_VENDOR | CTRL_RECIPIENT_DEVICE;
            let request = USB_REQUEST;
            let value = cmd_value;
            let index = USB_VALUE_INDEX;

            // Send the control transfer
            let result = self.device_handle.write_control(
                request_type,
                request,
                value,
                index,
                &data,
                USB_TIMEOUT,
            );

            match result {
                Ok(bytes_written) => {
                    log::debug!(
                        "LED command {} sent successfully ({} bytes)",
                        cmd_value,
                        bytes_written
                    );
                    Ok(())
                }
                Err(e) => {
                    log::error!("Failed to send LED command {}: {}", cmd_value, e);
                    Err(LedRingError::CommandFailed(e))
                }
            }
        }

        /// Convenience methods for common operations

        /// Turn off all LEDs
        pub fn off(&self) -> Result<(), LedRingError> {
            self.send_command(LedCommand::Mono {
                red: 0,
                green: 0,
                blue: 0,
            })
        }

        /// Set all LEDs to a single color
        pub fn set_color(&self, red: u8, green: u8, blue: u8) -> Result<(), LedRingError> {
            self.send_command(LedCommand::Mono { red, green, blue })
        }

        /// Set brightness (0-31)
        pub fn set_brightness(&self, brightness: u8) -> Result<(), LedRingError> {
            self.send_command(LedCommand::SetBrightness { brightness })
        }

        /// Activate listening mode (good for wake word detection)
        pub fn listen_mode(&self) -> Result<(), LedRingError> {
            self.send_command(LedCommand::Listen)
        }

        /// Activate speaking mode
        pub fn speak_mode(&self) -> Result<(), LedRingError> {
            self.send_command(LedCommand::Speak)
        }

        /// Show volume level (0-12)
        pub fn show_volume(&self, level: u8) -> Result<(), LedRingError> {
            self.send_command(LedCommand::ShowVolume { volume: level })
        }

        /// Create a breathing effect
        pub fn breathing_effect(
            &self,
            red: u8,
            green: u8,
            blue: u8,
            cycles: u32,
        ) -> Result<(), LedRingError> {
            for _ in 0..cycles {
                for brightness in (1..=20).chain((1..=20).rev()) {
                    self.set_brightness(brightness)?;
                    self.set_color(red, green, blue)?;
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            Ok(())
        }

        /// Create a rotating effect
        pub fn rotate_effect(
            &self,
            red: u8,
            green: u8,
            blue: u8,
            rotations: u32,
        ) -> Result<(), LedRingError> {
            for _ in 0..rotations {
                for led_pos in 0..12 {
                    let mut leds = [(0u8, 0u8, 0u8); 12];
                    leds[led_pos] = (red, green, blue);
                    // Add trailing LEDs for smooth effect
                    if led_pos > 0 {
                        leds[led_pos - 1] = (red / 3, green / 3, blue / 3);
                    }
                    if led_pos > 1 {
                        leds[led_pos - 2] = (red / 6, green / 6, blue / 6);
                    }

                    self.send_command(LedCommand::Custom { leds })?;
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            Ok(())
        }
    }

    impl Drop for LedRing {
        fn drop(&mut self) {
            // Turn off LEDs when dropping the controller
            let _ = self.off();
        }
    }
} // End of led_ring_impl module

// Re-export when led_ring feature is enabled
#[cfg(feature = "led_ring")]
pub use led_ring_impl::*;

// Provide stub implementations when led_ring feature is disabled
#[cfg(not(feature = "led_ring"))]
pub mod stub {
    use thiserror::Error;

    #[derive(Error, Debug)]
    pub enum LedRingError {
        #[error("LED ring support not compiled in")]
        NotSupported,
    }

    #[derive(Debug, Clone)]
    pub enum LedCommand {
        Trace,
        Mono {
            red: u8,
            green: u8,
            blue: u8,
        },
        Listen,
        Wait,
        Speak,
        Spin,
        Custom {
            leds: [(u8, u8, u8); 12],
        },
        SetBrightness {
            brightness: u8,
        },
        SetColorPalette {
            color1: (u8, u8, u8),
            color2: (u8, u8, u8),
        },
        SetCenterLed {
            mode: u8,
        },
        ShowVolume {
            volume: u8,
        },
    }

    pub struct LedRing;

    impl LedRing {
        pub fn new() -> Result<Self, LedRingError> {
            Err(LedRingError::NotSupported)
        }

        pub fn send_command(&self, _command: LedCommand) -> Result<(), LedRingError> {
            Err(LedRingError::NotSupported)
        }

        pub fn off(&self) -> Result<(), LedRingError> {
            Err(LedRingError::NotSupported)
        }
        pub fn set_color(&self, _red: u8, _green: u8, _blue: u8) -> Result<(), LedRingError> {
            Err(LedRingError::NotSupported)
        }
        pub fn set_brightness(&self, _brightness: u8) -> Result<(), LedRingError> {
            Err(LedRingError::NotSupported)
        }
        pub fn listen_mode(&self) -> Result<(), LedRingError> {
            Err(LedRingError::NotSupported)
        }
        pub fn speak_mode(&self) -> Result<(), LedRingError> {
            Err(LedRingError::NotSupported)
        }
        pub fn show_volume(&self, _level: u8) -> Result<(), LedRingError> {
            Err(LedRingError::NotSupported)
        }
        pub fn breathing_effect(
            &self,
            _red: u8,
            _green: u8,
            _blue: u8,
            _cycles: u32,
        ) -> Result<(), LedRingError> {
            Err(LedRingError::NotSupported)
        }
        pub fn rotate_effect(
            &self,
            _red: u8,
            _green: u8,
            _blue: u8,
            _rotations: u32,
        ) -> Result<(), LedRingError> {
            Err(LedRingError::NotSupported)
        }
    }
}

#[cfg(not(feature = "led_ring"))]
pub use stub::*;
