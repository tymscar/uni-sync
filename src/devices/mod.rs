use serde_derive::{Deserialize, Serialize};
use hidapi::{self, HidDevice};
use std::{thread, time};

mod control_transfer;

#[derive(Serialize, Deserialize, Clone)]
pub struct Configs {
    pub configs: Vec<Config>
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    pub device_id: String,
    pub sync_rgb: bool,
    pub channels: Vec<Channel>
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Channel {
    pub mode: String,
    pub speed: usize,
}

const VENDOR_IDS: [u16; 1] = [ 0x0cf2 ];
const PRODUCT_IDS: [u16; 7] = [ 0x7750, 0xa100, 0xa101, 0xa102, 0xa103, 0xa104, 0xa105 ];

const V1_2_PRODUCT_IDS: [u16; 2] = [ 0x7750, 0xa100 ];

enum TransportMode {
    HID,
    VendorControl,
}

fn percent_to_rpm(pct: u8, product_id: u16) -> u16 {
    let pct = pct.min(100) as u32;
    
    let (min_rpm, max_rpm) = match product_id {
        0xa100 => (500u32, 1500u32),
        0x7750 => (800u32, 1900u32),
        0xa101 => (800u32, 1900u32),
        0xa102 => (200u32, 2100u32),
        0xa103 | 0xa105 | 0xa104 => (250u32, 2000u32),
        _ => (800u32, 1900u32),
    };
    
    (min_rpm + ((max_rpm - min_rpm) * pct / 100)) as u16
}

fn detect_transport_mode(product_id: u16) -> TransportMode {
    if V1_2_PRODUCT_IDS.contains(&product_id) {
        TransportMode::VendorControl
    } else {
        TransportMode::HID
    }
}

pub fn run(mut existing_configs: Configs) -> Configs {

    let mut default_channels: Vec<Channel> = Vec::new();
    for _x in 0..4 {
        default_channels.push(Channel {
            mode: "Manual".to_string(),
            speed: 50
        });
    }

    // Get All Devices
    let api = match hidapi::HidApi::new() {
        Ok(api) => api,
        Err(_) => panic!("Could not find any controllers")
    };
    
    for hiddevice in api.device_list() {
        if VENDOR_IDS.contains(&hiddevice.vendor_id()) && PRODUCT_IDS.contains(&hiddevice.product_id()) {

            let serial_number: &str = match hiddevice.serial_number() {
                Some(sn) => sn,
                None => {
                    println!("Serial number not available for device {:?}", hiddevice);
                    continue; 
                }
            };

            let path: &str = match hiddevice.path() {
                p => p.to_str().unwrap_or("unknown"),
            };

            let device_id: String = format!("VID:{}/PID:{}/SN:{}/PATH:{}", hiddevice.vendor_id().to_string(), hiddevice.product_id().to_string(), serial_number.to_string(), path.to_string());
            let hid: HidDevice = match api.open_path(hiddevice.path()) {
                Ok(hid) => hid,
                Err(_) => {
                    println!("Please run uni-sync with elevated permissions.");
                    std::process::exit(0);
                }
            };
            let mut channels: Vec<Channel> = default_channels.clone();
            let mut sync_rgb: bool = false;


            println!("Found: {:?}", device_id);

            if let Some(config) = existing_configs.configs.iter().find( | config | config.device_id == device_id) {
                channels = config.channels.clone();
                sync_rgb = config.sync_rgb;
            } else {
                existing_configs.configs.push(Config {
                    device_id: device_id,
                    sync_rgb: false,
                    channels: channels.clone()
                });
            }

            
            // Send Command to Sync to RGB Header
            let sync_byte: u8 = if sync_rgb { 1 } else { 0 };
            let _ = match &hiddevice.product_id() {
                0xa100|0x7750 => hid.write(&[224, 16, 48, sync_byte, 0, 0, 0]), // SL
                0xa101 => hid.write(&[224, 16, 65, sync_byte, 0, 0, 0]), // AL
                0xa102 => hid.write(&[224, 16, 97, sync_byte, 0, 0, 0]), // SLI
                0xa103|0xa105 => hid.write(&[224, 16, 97, sync_byte, 0, 0, 0]), // SLv2
                0xa104 => hid.write(&[224, 16, 97, sync_byte, 0, 0, 0]), // ALv2
                _ => hid.write(&[224, 16, 48, sync_byte, 0, 0, 0]), // SL
            };

            // Avoid Race Condition
            thread::sleep(time::Duration::from_millis(200));


            let transport_mode = detect_transport_mode(hiddevice.product_id());
            
            match transport_mode {
                TransportMode::VendorControl => {
                    apply_vendor_control_settings(
                        hiddevice.vendor_id(),
                        hiddevice.product_id(),
                        serial_number,
                        &channels,
                        &hid,
                        &hiddevice
                    );
                }
                TransportMode::HID => {
                    println!("Using HID mode for device");
                    apply_hid_settings(&hid, &hiddevice, &channels);
                }
            }
        }
    }
    return existing_configs;
}

fn apply_vendor_control_settings(
    vendor_id: u16,
    product_id: u16,
    serial_number: &str,
    channels: &Vec<Channel>,
    hid: &HidDevice,
    hiddevice: &hidapi::DeviceInfo
) {
    println!("Using VendorControl mode for device");
    
    if let Ok(vendor_device) = control_transfer::VendorDevice::open(
        vendor_id,
        product_id,
        serial_number
    ) {
        for x in 0..channels.len() {
            if channels[x].mode == "Manual" {
                let speed_pct = channels[x].speed.min(100) as u8;
                let rpm = percent_to_rpm(speed_pct, product_id);
                
                if let Err(e) = vendor_device.set_speed_with_delays(x as u8, rpm) {
                    eprintln!("Failed to set channel {} speed: {:?}", x, e);
                } else {
                    println!("Set channel {} to {}% ({} RPM)", x, speed_pct, rpm);
                }
            }
        }
    } else {
        eprintln!("Failed to open device via VendorControl, falling back to HID");
        apply_hid_settings(hid, hiddevice, channels);
    }
}

fn apply_hid_settings(hid: &HidDevice, hiddevice: &hidapi::DeviceInfo, channels: &Vec<Channel>) {
    for x in 0..channels.len() {
        let mut channel_byte = 0x10 << x;

        if channels[x].mode == "PWM" {
            channel_byte = channel_byte | 0x1 << x;
        }

        let _ = match &hiddevice.product_id() {
            0xa100|0x7750 => hid.write(&[224, 16, 49, channel_byte]), // SL
            0xa101 => hid.write(&[224, 16, 66, channel_byte]), // AL
            0xa102 => hid.write(&[224, 16, 98, channel_byte]), // SLI
            0xa103|0xa105 => hid.write(&[224, 16, 98, channel_byte]), // SLv2
            0xa104 => hid.write(&[224, 16, 98, channel_byte]), // ALv2
            _ => hid.write(&[224, 16, 49, channel_byte]), // SL
        };

        // Avoid Race Condition
        thread::sleep(time::Duration::from_millis(200));

        // Set Channel Speed
        if channels[x].mode == "Manual" {
            let mut speed = channels[x].speed as f64;
            if speed > 100.0 { speed = 100.0 }

            let speed_800_1900: u8 = ((800.0 + (11.0 * speed)) as usize / 19).try_into().unwrap();
            let speed_250_2000: u8 = ((250.0 + (17.5 * speed)) as usize / 20).try_into().unwrap();
            let speed_200_2100: u8 = ((200.0 + (19.0 * speed)) as usize  / 21).try_into().unwrap();

            let _ = match &hiddevice.product_id() {
                0xa100|0x7750 => hid.write(&[224, (x+32).try_into().unwrap(), 0, speed_800_1900]), // SL
                0xa101 => hid.write(&[224, (x+32).try_into().unwrap(), 0, speed_800_1900]), // AL
                0xa102 => hid.write(&[224, (x+32).try_into().unwrap(), 0, speed_200_2100]), // SLI
                0xa103|0xa105 => hid.write(&[224, (x+32).try_into().unwrap(), 0, speed_250_2000]), // SLv2
                0xa104 => hid.write(&[224, (x+32).try_into().unwrap(), 0, speed_250_2000]), // ALv2
                _ => hid.write(&[224, (x+32).try_into().unwrap(), 0, speed_800_1900]), // SL
            };

            // Avoid Race Condition
            thread::sleep(time::Duration::from_millis(100));
        }
    }
}
