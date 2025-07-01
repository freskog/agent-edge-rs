use cpal::traits::{DeviceTrait, HostTrait};

fn main() {
    let host = cpal::default_host();

    println!("Default input device:");
    if let Some(device) = host.default_input_device() {
        if let Ok(name) = device.name() {
            println!("  Name: {}", name);
        }
        if let Ok(config) = device.default_input_config() {
            println!("  Default config:");
            println!("    Sample format: {:?}", config.sample_format());
            println!("    Channels: {}", config.channels());
            println!("    Sample rate: {}", config.sample_rate().0);
        }
        if let Ok(configs) = device.supported_input_configs() {
            println!("  Supported configs:");
            for config in configs {
                println!(
                    "    Format: {:?}, channels: {}, rate: {}-{}",
                    config.sample_format(),
                    config.channels(),
                    config.min_sample_rate().0,
                    config.max_sample_rate().0
                );
            }
        }
    }

    println!("\nDefault output device:");
    if let Some(device) = host.default_output_device() {
        if let Ok(name) = device.name() {
            println!("  Name: {}", name);
        }
        if let Ok(config) = device.default_output_config() {
            println!("  Default config:");
            println!("    Sample format: {:?}", config.sample_format());
            println!("    Channels: {}", config.channels());
            println!("    Sample rate: {}", config.sample_rate().0);
        }
        if let Ok(configs) = device.supported_output_configs() {
            println!("  Supported configs:");
            for config in configs {
                println!(
                    "    Format: {:?}, channels: {}, rate: {}-{}",
                    config.sample_format(),
                    config.channels(),
                    config.min_sample_rate().0,
                    config.max_sample_rate().0
                );
            }
        }
    }

    println!("\nAll input devices:");
    if let Ok(devices) = host.devices() {
        for device in devices {
            if device.default_input_config().is_ok() {
                if let Ok(name) = device.name() {
                    println!("\nDevice: {}", name);
                    if let Ok(config) = device.default_input_config() {
                        println!("  Default format: {:?}", config.sample_format());
                        println!("  Channels: {}", config.channels());
                        println!("  Sample rate: {}", config.sample_rate().0);
                    }
                }
            }
        }
    }

    println!("\nAll output devices:");
    if let Ok(devices) = host.devices() {
        for device in devices {
            if device.default_output_config().is_ok() {
                if let Ok(name) = device.name() {
                    println!("\nDevice: {}", name);
                    if let Ok(config) = device.default_output_config() {
                        println!("  Default format: {:?}", config.sample_format());
                        println!("  Channels: {}", config.channels());
                        println!("  Sample rate: {}", config.sample_rate().0);
                    }
                    if let Ok(configs) = device.supported_output_configs() {
                        println!("  Supported configs:");
                        for config in configs {
                            println!(
                                "    Format: {:?}, channels: {}, rate: {}-{}",
                                config.sample_format(),
                                config.channels(),
                                config.min_sample_rate().0,
                                config.max_sample_rate().0
                            );
                        }
                    }
                }
            }
        }
    }
}
