use agent_edge_rs::led_ring::LedRing;
use std::{thread, time::Duration};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing ReSpeaker LED Ring Connection...");

    // Try to initialize the LED ring
    let led_ring = LedRing::new()?;
    println!("✓ LED ring connected successfully!");

    // Test basic functionality
    println!("Testing basic LED functionality...");

    // Red
    println!("Setting red...");
    led_ring.set_color(255, 0, 0)?;
    thread::sleep(Duration::from_secs(1));

    // Green
    println!("Setting green...");
    led_ring.set_color(0, 255, 0)?;
    thread::sleep(Duration::from_secs(1));

    // Blue
    println!("Setting blue...");
    led_ring.set_color(0, 0, 255)?;
    thread::sleep(Duration::from_secs(1));

    // Turn off
    println!("Turning off...");
    led_ring.off()?;

    println!("✓ LED test completed successfully!");
    println!();
    println!("Your ReSpeaker LED ring is working correctly!");
    println!("You can now use LED feedback in your wake word detection application.");

    Ok(())
}
