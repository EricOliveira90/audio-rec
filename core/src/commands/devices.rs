use crate::error::Result;

#[cfg(not(windows))]
use serde_json::json;

/// Execute the devices command - enumerate and display all audio devices
pub async fn execute() -> Result<()> {
    #[cfg(windows)]
    {
        use crate::wasapi::device_enumeration::{enumerate_devices, DeviceDirection, DeviceList};

        let input_devices = enumerate_devices(DeviceDirection::Input)?;
        let output_devices = enumerate_devices(DeviceDirection::Output)?;

        let device_list = DeviceList::new(input_devices, output_devices);
        let json_str = serde_json::to_string(&device_list)?;
        println!("{}", json_str);
    }

    #[cfg(not(windows))]
    {
        let result = json!({
            "type": "error",
            "message": "Device enumeration is currently only supported on Windows. Cross-platform support is planned for a future release."
        });
        println!("{}", serde_json::to_string(&result)?);
    }

    Ok(())
}
