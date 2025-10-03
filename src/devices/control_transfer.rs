use rusb::{Context, DeviceHandle, UsbContext};
use std::{thread, time::Duration};

const VENDOR_REQUEST_TYPE: u8 = 0x40;
const VENDOR_REQUEST: u8 = 128;
const TIMEOUT_MS: u64 = 1000;
const INTER_CMD_DELAY_MS: u64 = 100;

pub struct VendorDevice {
    handle: DeviceHandle<Context>,
}

impl VendorDevice {
    pub fn open(vid: u16, pid: u16, serial: &str) -> Result<Self, rusb::Error> {
        let context = Context::new()?;
        
        for device in context.devices()?.iter() {
            let device_desc = device.device_descriptor()?;
            
            if device_desc.vendor_id() == vid && device_desc.product_id() == pid {
                let handle = device.open()?;
                
                if let Ok(sn) = handle.read_serial_number_string_ascii(&device_desc) {
                    if sn == serial {
                        handle.set_auto_detach_kernel_driver(true).ok();
                        
                        if let Err(e) = handle.claim_interface(0) {
                            eprintln!("Failed to claim interface 0: {:?}", e);
                            return Err(e);
                        }
                        
                        return Ok(VendorDevice { handle });
                    }
                }
            }
        }
        
        Err(rusb::Error::NoDevice)
    }
    
    pub fn setup_channel(&self, channel_id: u8) -> Result<(), rusb::Error> {
        let channel_mask: u16 = 0x10 << channel_id;
        
        let mut payload = [0u8; 16];
        payload[2] = (channel_mask & 0xFF) as u8;
        payload[3] = ((channel_mask >> 8) & 0xFF) as u8;
        payload[15] = 0x01;
        
        self.handle.write_control(
            VENDOR_REQUEST_TYPE,
            VENDOR_REQUEST,
            0,
            0xe020,
            &payload,
            Duration::from_millis(TIMEOUT_MS)
        )?;
        
        Ok(())
    }
    
    pub fn set_channel_speed(&self, channel_id: u8, rpm: u16) -> Result<(), rusb::Error> {
        let windex: u16 = 0xd8a0 + (channel_id as u16 * 2);
        
        let payload: [u8; 2] = [
            (rpm & 0xFF) as u8,
            ((rpm >> 8) & 0xFF) as u8
        ];
        
        self.handle.write_control(
            VENDOR_REQUEST_TYPE,
            VENDOR_REQUEST,
            0,
            windex,
            &payload,
            Duration::from_millis(TIMEOUT_MS)
        )?;
        
        Ok(())
    }
    
    pub fn commit_channel(&self, channel_id: u8) -> Result<(), rusb::Error> {
        let windex: u16 = 0xd890 + channel_id as u16;
        let payload: [u8; 1] = [0x01];
        
        self.handle.write_control(
            VENDOR_REQUEST_TYPE,
            VENDOR_REQUEST,
            0,
            windex,
            &payload,
            Duration::from_millis(TIMEOUT_MS)
        )?;
        
        Ok(())
    }
    
    pub fn set_speed_with_delays(&self, channel_id: u8, rpm: u16) -> Result<(), rusb::Error> {
        self.setup_channel(channel_id)?;
        thread::sleep(Duration::from_millis(INTER_CMD_DELAY_MS));
        
        self.set_channel_speed(channel_id, rpm)?;
        thread::sleep(Duration::from_millis(INTER_CMD_DELAY_MS));
        
        self.commit_channel(channel_id)?;
        
        Ok(())
    }
}

impl Drop for VendorDevice {
    fn drop(&mut self) {
        let _ = self.handle.release_interface(0);
    }
}
