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
const BUTTON_GPIO: u8 = 10;

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

    let (state_sender, system_state) = watch::channel(SystemState::NoSdCard);
    let driver = LedDriver::new(red, yellow, system_state.clone());
    let _led_jh = tokio::spawn(async move { driver.update_loop().await });

    let button_gpio = Gpio::new()?.get(BUTTON_GPIO)?.into_input_pullup();

    let (sender, mut button_receiver) = watch::channel(());
    button_receiver.mark_unchanged();
    let _button_jh = tokio::spawn(async move {
        let mut last_state = button_gpio.is_low();
        loop {
            tokio::time::sleep(Duration::from_millis(25)).await;
            // Button is pressed.
            let current_state = button_gpio.is_low();

            if [last_state, current_state] == [false, true] {
                sender.send_replace(());
            }
            last_state = current_state;
        }
    });

    let mut device_path = None;

    loop {
        tokio::time::sleep(Duration::from_millis(50)).await;
        match system_state.borrow().clone() {
            SystemState::NoSdCard => {
                //Get all devices that are at least 128 GB
                let devices = get_block_devices_with_size(128 * 1000 * 1000 * 1000);
                let Ok(devices) = devices else {
                    println!(
                        "Got error when querying devices: {:?}",
                        devices.unwrap_err()
                    );
                    continue;
                };
                println!("Have devices: {devices:?}");
                device_path = devices.get(0).cloned();
                state_sender.send_replace(SystemState::SdCardFound);
            }
            SystemState::SdCardFound => {
                let Some(ref device_path) = device_path else {
                    state_sender.send_replace(SystemState::NoSdCard);
                    continue;
                };
                println!("Have device! {device_path:?}");
            }
            SystemState::Flashing => {}
            SystemState::FlashingFailed => {}
            SystemState::FlashingSuceeded => {}
        };
    }
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
use std::path::{Path, PathBuf};

fn get_block_devices_with_size(min_size_bytes: u64) -> io::Result<Vec<PathBuf>> {
    let block_path = Path::new("/sys/block");

    Ok(fs::read_dir(block_path)?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path().join("size");
            if path.exists() {
                let size = fs::read_to_string(&path).ok()?.trim().to_string();
                match size.parse::<u64>() {
                    Ok(size_blocks) => Some((entry, size_blocks * 512)),
                    Err(error) => {
                        println!("Got error when parsing path: {entry:?}. Error={error:?}");
                        None
                    }
                }
            } else {
                None
            }
        })
        .filter_map(|(entry, size)| {
            if size < min_size_bytes {
                None
            } else {
                Some(entry.path())
            }
        })
        .collect())
}
