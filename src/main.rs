// gpio_blinkled.rs - Blinks an LED in a loop.
//
// Remember to add a resistor of an appropriate value in series, to prevent
// exceeding the maximum current rating of the GPIO pin and the LED.
//
// Interrupting the process by pressing Ctrl-C causes the application to exit
// immediately without resetting the pin's state, so the LED might stay lit.
// Check out the gpio_blinkled_signals.rs example to learn how to properly
// handle incoming signals to prevent an abnormal termination.

use std::error::Error;
use std::thread;
use std::time::{Duration, Instant};

use std::fs::File;
use std::io::{self, copy, BufReader, BufWriter};

use rppal::gpio::Gpio;

type WhateverResult = Result<(), Box<dyn Error + Send>>;

// Gpio uses BCM pin numbering. BCM GPIO 23 is tied to physical pin 16.
const LED_YELLOW: u8 = 27;
const LED_RED: u8 = 23;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SystemState {
    /// An SD card needs to be inserted
    NoSdCard,
    /// We found an SD card
    SdCardFound,
    /// Flashing in progress
    Flashing,
    /// Flashing is nominal (image checksum matches)
    FlashingSuceeded,
    /// Flashing failed (image checksum doesn't match)
    FlashingFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LedState {
    Off,
    SolidBoth,
    FlashingGreen,
    FlashingRed,
    FlashingGreenRed,
    SolidGreen,
    SolidRed,
}

impl Into<LedState> for SystemState {
    fn into(self) -> LedState {
        match self {
            Self::NoSdCard => LedState::FlashingRed,
            Self::SdCardFound => LedState::FlashingGreen,
            Self::Flashing => LedState::FlashingGreenRed,
            Self::FlashingSuceeded => LedState::SolidGreen,
            Self::FlashingFailed => LedState::SolidRed,
        }
    }
}

use rppal::gpio::OutputPin;
use tokio::sync::watch;

struct LedDriver {
    red: OutputPin,
    yellow: OutputPin,
    receiver: watch::Receiver<SystemState>,
}

impl LedDriver {
    fn new(red: OutputPin, yellow: OutputPin, receiver: watch::Receiver<SystemState>) -> Self {
        Self {
            red,
            yellow,
            receiver,
        }
    }

    async fn update_loop(mut self) -> WhateverResult {
        let LedDriver {
            ref mut red,
            ref mut yellow,
            mut receiver,
        } = self;
        let mut flash_state = false;
        let mut led_state = LedState::SolidBoth;
        let mut timer = tokio::time::interval(Duration::from_millis(300));

        let set_output = |led: &mut OutputPin, state: bool| {
            if state {
                led.set_low();
            } else {
                led.set_high();
            }
        };

        loop {
            tokio::select! {
                _ = receiver.changed() => {
                    led_state = receiver.borrow_and_update().clone().into();
                    flash_state = false;
                }
                _ = timer.tick() => {
                    flash_state = !flash_state;
                }
            }
            match (led_state, flash_state) {
                (LedState::Off, _) => {
                    set_output(red, false);
                    set_output(yellow, false);
                }
                (LedState::SolidBoth, _) => {
                    set_output(red, true);
                    set_output(yellow, true);
                }
                (LedState::SolidRed, _) => {
                    set_output(red, true);
                    set_output(yellow, false);
                }
                (LedState::SolidGreen, _) => {
                    set_output(red, false);
                    set_output(yellow, true);
                }
                (LedState::FlashingGreenRed, flash_state) => {
                    set_output(red, flash_state);
                    set_output(yellow, !flash_state);
                }
                (LedState::FlashingGreen, flash_state) => {
                    set_output(yellow, flash_state);
                    set_output(red, false);
                }
                (LedState::FlashingRed, flash_state) => {
                    set_output(red, flash_state);
                    set_output(yellow, false);
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let red = Gpio::new()?.get(LED_RED)?.into_output();
    let yellow = Gpio::new()?.get(LED_YELLOW)?.into_output();

    let (sender, receiver) = watch::channel(SystemState::NoSdCard);

    let driver = LedDriver::new(red, yellow, receiver.clone());
    let jh = tokio::spawn(async move { driver.update_loop().await });

    let system_state = [
        SystemState::NoSdCard,
        SystemState::SdCardFound,
        SystemState::Flashing,
        SystemState::FlashingSuceeded,
        SystemState::FlashingFailed,
    ];

    let mut state = system_state.into_iter();

    loop {
        sender.send(state.next().ok_or("Ran out of values")?)?;
        println!("Current state is {:?}", receiver.borrow());
        tokio::time::sleep(Duration::from_millis(2000)).await;
    }

    Ok(())
}

/*
fn main() -> Result<(), Box<dyn Error>> {
    let input = File::open("disk.img")?;
    let output = File::options().write(true).open("/dev/sdX")?; // replace with actual device

    let mut reader = BufReader::new(input);
    let mut writer = BufWriter::new(output);

    copy(&mut reader, &mut writer)?;

    // Retrieve the GPIO pin and configure it as an output.
    let mut pin = Gpio::new()?.get(GPIO_LED)?.into_output();

    loop {
        pin.toggle();
        thread::sleep(Duration::from_millis(500));
    }
}
*/
use std::fs;
use std::path::Path;
use tokio::time::Interval;

fn get_block_devices_with_size(min_size_bytes: u64) -> io::Result<()> {
    let block_path = Path::new("/sys/block");

    for entry in fs::read_dir(block_path)? {
        let entry = entry?;
        let dev_name = entry.file_name().into_string().unwrap();
        let size_path = entry.path().join("size");

        if size_path.exists() {
            let size_str = fs::read_to_string(&size_path)?.trim().to_string();

            if let Ok(sectors) = size_str.parse::<u64>() {
                let size_bytes = sectors * 512;

                if size_bytes >= min_size_bytes {
                    println!(
                        "/dev/{} - {} bytes ({:.2} GB)",
                        dev_name,
                        size_bytes,
                        size_bytes as f64 / 1_073_741_824.0
                    );
                }
            }
        }
    }

    Ok(())
}
