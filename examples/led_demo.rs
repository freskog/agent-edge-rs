use agent_edge_rs::led_ring::{LedCommand, LedRing};
use std::thread;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("ReSpeaker LED Ring Demo");

    // Initialize the LED ring controller
    let led_ring = match LedRing::new() {
        Ok(ring) => ring,
        Err(e) => {
            eprintln!("Failed to initialize LED ring: {}", e);
            eprintln!("Make sure your ReSpeaker 4-mic array is connected and accessible.");
            eprintln!("On Linux, you may need to run with sudo or add udev rules.");
            return Err(e.into());
        }
    };

    println!("LED ring initialized successfully!");

    // Demo sequence
    println!("1. Setting brightness to medium...");
    led_ring.set_brightness(15)?;
    thread::sleep(Duration::from_secs(1));

    println!("2. Red color for 2 seconds...");
    led_ring.set_color(255, 0, 0)?;
    thread::sleep(Duration::from_secs(2));

    println!("3. Green color for 2 seconds...");
    led_ring.set_color(0, 255, 0)?;
    thread::sleep(Duration::from_secs(2));

    println!("4. Blue color for 2 seconds...");
    led_ring.set_color(0, 0, 255)?;
    thread::sleep(Duration::from_secs(2));

    println!("5. Purple breathing effect...");
    led_ring.breathing_effect(128, 0, 128, 3)?;

    println!("6. Rotating blue effect...");
    led_ring.rotate_effect(0, 100, 255, 3)?;

    println!("7. Listen mode (for wake word detection)...");
    led_ring.listen_mode()?;
    thread::sleep(Duration::from_secs(3));

    println!("8. Speak mode...");
    led_ring.speak_mode()?;
    thread::sleep(Duration::from_secs(3));

    println!("9. Volume display (simulating levels 0-12)...");
    for level in 0..=12 {
        led_ring.show_volume(level)?;
        thread::sleep(Duration::from_millis(200));
    }
    for level in (0..=12).rev() {
        led_ring.show_volume(level)?;
        thread::sleep(Duration::from_millis(200));
    }

    println!("10. Custom LED pattern (rainbow)...");
    let rainbow_colors = [
        (255, 0, 0),   // Red
        (255, 127, 0), // Orange
        (255, 255, 0), // Yellow
        (127, 255, 0), // Yellow-green
        (0, 255, 0),   // Green
        (0, 255, 127), // Green-cyan
        (0, 255, 255), // Cyan
        (0, 127, 255), // Cyan-blue
        (0, 0, 255),   // Blue
        (127, 0, 255), // Blue-magenta
        (255, 0, 255), // Magenta
        (255, 0, 127), // Magenta-red
    ];

    led_ring.send_command(LedCommand::Custom {
        leds: rainbow_colors,
    })?;
    thread::sleep(Duration::from_secs(3));

    println!("11. Spin mode...");
    led_ring.send_command(LedCommand::Spin)?;
    thread::sleep(Duration::from_secs(3));

    println!("12. Turning off LEDs...");
    led_ring.off()?;

    println!("Demo complete!");
    Ok(())
}
