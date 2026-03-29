//! WASAPI Device Enumeration Module
//!
//! Encapsulates all COM/WASAPI device enumeration complexity behind a simple interface.
//! Provides functions to list audio devices and resolve device indices to WASAPI device handles.

use serde::{Deserialize, Serialize};

use crate::error::{RecorderError, Result};

/// Represents an audio device with its properties
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioDevice {
    pub index: u32,
    pub name: String,
    pub is_default: bool,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Direction of audio flow for device enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceDirection {
    Input,  // Capture devices (microphones)
    Output, // Render devices (speakers/headphones)
}

/// Result of device enumeration containing both input and output devices
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceList {
    #[serde(rename = "type")]
    pub event_type: String,
    pub input_devices: Vec<AudioDevice>,
    pub output_devices: Vec<AudioDevice>,
}

impl DeviceList {
    pub fn new(input_devices: Vec<AudioDevice>, output_devices: Vec<AudioDevice>) -> Self {
        Self {
            event_type: "devices".to_string(),
            input_devices,
            output_devices,
        }
    }
}

/// Validate that a device index is within bounds for a given device list
pub fn validate_device_index(
    index: u32,
    devices: &[AudioDevice],
    direction: DeviceDirection,
) -> Result<&AudioDevice> {
    devices.get(index as usize).ok_or_else(|| {
        let direction_str = match direction {
            DeviceDirection::Input => "input",
            DeviceDirection::Output => "output",
        };
        RecorderError::InvalidParameter(format!(
            "Invalid {} device index {}. Available {} devices: 0-{}",
            direction_str,
            index,
            devices.len(),
            if devices.is_empty() {
                0
            } else {
                devices.len() - 1
            }
        ))
    })
}

/// Enumerate all audio devices on the system (Windows-only)
#[cfg(windows)]
pub fn enumerate_devices(direction: DeviceDirection) -> Result<Vec<AudioDevice>> {
    use windows::Win32::System::Com::*;

    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|e| RecorderError::Other(anyhow::anyhow!("Failed to initialize COM: {}", e)))?;

        let result = enumerate_devices_inner(direction);

        CoUninitialize();

        result
    }
}

#[cfg(windows)]
unsafe fn enumerate_devices_inner(direction: DeviceDirection) -> Result<Vec<AudioDevice>> {
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::*;
    use windows::Win32::UI::Shell::PropertiesSystem::*;
    use windows::core::GUID;

    let device_enumerator: IMMDeviceEnumerator =
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
            .map_err(|e| RecorderError::Other(anyhow::anyhow!("Failed to create device enumerator: {}", e)))?;

    let data_flow = match direction {
        DeviceDirection::Input => eCapture,
        DeviceDirection::Output => eRender,
    };

    // Get default device for comparison
    let default_device_id = device_enumerator
        .GetDefaultAudioEndpoint(data_flow, eConsole)
        .ok()
        .and_then(|device| device.GetId().ok())
        .map(|id| id.to_string().unwrap_or_default());

    // Enumerate all devices
    let collection = device_enumerator
        .EnumAudioEndpoints(data_flow, DEVICE_STATE_ACTIVE)
        .map_err(|e| RecorderError::Other(anyhow::anyhow!("Failed to enumerate devices: {}", e)))?;

    let count = collection
        .GetCount()
        .map_err(|e| RecorderError::Other(anyhow::anyhow!("Failed to get device count: {}", e)))?;

    let mut devices = Vec::new();

    // PKEY_Device_FriendlyName: {a45c254e-df1c-4efd-8020-67d146a850e0}, 14
    let pkey_device_friendly_name = PROPERTYKEY {
        fmtid: GUID::from_values(
            0xa45c254e,
            0xdf1c,
            0x4efd,
            [0x80, 0x20, 0x67, 0xd1, 0x46, 0xa8, 0x50, 0xe0],
        ),
        pid: 14,
    };

    for i in 0..count {
        let device = match collection.Item(i) {
            Ok(d) => d,
            Err(_) => continue,
        };

        // Get device ID
        let device_id = device
            .GetId()
            .ok()
            .map(|id| id.to_string().unwrap_or_default())
            .unwrap_or_default();

        // Get friendly name via property store
        let name = get_device_friendly_name(&device, &pkey_device_friendly_name, i);

        // Check if this is the default device
        let is_default = default_device_id
            .as_ref()
            .map(|default_id| *default_id == device_id)
            .unwrap_or(false);

        // Get sample rate and channels via IAudioClient
        let (sample_rate, channels) = get_device_format(&device);

        devices.push(AudioDevice {
            index: i,
            name,
            is_default,
            sample_rate,
            channels,
        });
    }

    Ok(devices)
}

/// Get the friendly name of a device from its property store
#[cfg(windows)]
unsafe fn get_device_friendly_name(
    device: &windows::Win32::Media::Audio::IMMDevice,
    pkey: &windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY,
    fallback_index: u32,
) -> String {
    use windows::Win32::System::Com::StructuredStorage::*;
    use windows::Win32::System::Com::*;

    device
        .OpenPropertyStore(STGM_READ)
        .ok()
        .and_then(|store| store.GetValue(pkey).ok())
        .and_then(|prop| {
            // Use PropVariantToStringAlloc to safely extract the string
            PropVariantToStringAlloc(&prop)
                .ok()
                .map(|pwsz| pwsz.to_string().unwrap_or_else(|_| format!("Device {}", fallback_index)))
        })
        .unwrap_or_else(|| format!("Device {}", fallback_index))
}

/// Get the sample rate and channel count of a device
#[cfg(windows)]
unsafe fn get_device_format(
    device: &windows::Win32::Media::Audio::IMMDevice,
) -> (u32, u16) {
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::*;

    let audio_client: std::result::Result<IAudioClient, _> = device.Activate(CLSCTX_ALL, None);
    match audio_client {
        Ok(client) => {
            match client.GetMixFormat() {
                Ok(mix_format) => {
                    let wf = &*mix_format;
                    let sr = wf.nSamplesPerSec;
                    let ch = wf.nChannels;
                    CoTaskMemFree(Some(mix_format as *const _ as *const _));
                    (sr, ch)
                }
                Err(_) => (0, 0),
            }
        }
        Err(_) => (0, 0),
    }
}

/// Get a specific device by index (Windows-only)
/// Returns the device name for inclusion in events
#[cfg(windows)]
pub fn get_device_by_index(
    direction: DeviceDirection,
    index: u32,
) -> Result<(windows::Win32::Media::Audio::IMMDevice, String)> {
    let devices = enumerate_devices(direction)?;
    let device_info = validate_device_index(index, &devices, direction)?;
    let device_name = device_info.name.clone();

    // Re-enumerate to get the actual IMMDevice handle
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::*;

    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|e| RecorderError::Other(anyhow::anyhow!("Failed to initialize COM: {}", e)))?;

        let device_enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|e| RecorderError::Other(anyhow::anyhow!("Failed to create device enumerator: {}", e)))?;

        let data_flow = match direction {
            DeviceDirection::Input => eCapture,
            DeviceDirection::Output => eRender,
        };

        let collection = device_enumerator
            .EnumAudioEndpoints(data_flow, DEVICE_STATE_ACTIVE)
            .map_err(|e| RecorderError::Other(anyhow::anyhow!("Failed to enumerate devices: {}", e)))?;

        let device = collection
            .Item(index)
            .map_err(|e| RecorderError::Other(anyhow::anyhow!("Failed to get device at index {}: {}", index, e)))?;

        CoUninitialize();

        Ok((device, device_name))
    }
}

/// Enumerate devices on non-Windows platforms (returns error)
#[cfg(not(windows))]
pub fn enumerate_devices(_direction: DeviceDirection) -> Result<Vec<AudioDevice>> {
    Err(RecorderError::Other(anyhow::anyhow!(
        "Device enumeration is currently only supported on Windows."
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── AudioDevice serialization ──

    #[test]
    fn test_audio_device_serializes_to_json_with_all_fields() {
        let device = AudioDevice {
            index: 0,
            name: "Microphone (Blue Yeti)".to_string(),
            is_default: true,
            sample_rate: 48000,
            channels: 2,
        };

        let json = serde_json::to_value(&device).unwrap();

        assert_eq!(json["index"], 0);
        assert_eq!(json["name"], "Microphone (Blue Yeti)");
        assert_eq!(json["is_default"], true);
        assert_eq!(json["sample_rate"], 48000);
        assert_eq!(json["channels"], 2);
    }

    #[test]
    fn test_audio_device_deserializes_from_json() {
        let json = r#"{
            "index": 1,
            "name": "Speakers (Realtek)",
            "is_default": false,
            "sample_rate": 44100,
            "channels": 2
        }"#;

        let device: AudioDevice = serde_json::from_str(json).unwrap();

        assert_eq!(device.index, 1);
        assert_eq!(device.name, "Speakers (Realtek)");
        assert!(!device.is_default);
        assert_eq!(device.sample_rate, 44100);
        assert_eq!(device.channels, 2);
    }

    #[test]
    fn test_audio_device_roundtrip_serialization() {
        let device = AudioDevice {
            index: 2,
            name: "HDMI Audio".to_string(),
            is_default: false,
            sample_rate: 48000,
            channels: 8,
        };

        let json_str = serde_json::to_string(&device).unwrap();
        let deserialized: AudioDevice = serde_json::from_str(&json_str).unwrap();

        assert_eq!(device, deserialized);
    }

    // ── DeviceList serialization ──

    #[test]
    fn test_device_list_serializes_with_type_field() {
        let list = DeviceList::new(
            vec![AudioDevice {
                index: 0,
                name: "Mic".to_string(),
                is_default: true,
                sample_rate: 48000,
                channels: 1,
            }],
            vec![AudioDevice {
                index: 0,
                name: "Speakers".to_string(),
                is_default: true,
                sample_rate: 48000,
                channels: 2,
            }],
        );

        let json = serde_json::to_value(&list).unwrap();

        assert_eq!(json["type"], "devices");
        assert_eq!(json["input_devices"].as_array().unwrap().len(), 1);
        assert_eq!(json["output_devices"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_device_list_with_empty_devices() {
        let list = DeviceList::new(vec![], vec![]);

        let json = serde_json::to_value(&list).unwrap();

        assert_eq!(json["type"], "devices");
        assert_eq!(json["input_devices"].as_array().unwrap().len(), 0);
        assert_eq!(json["output_devices"].as_array().unwrap().len(), 0);
    }

    // ── Index validation ──

    #[test]
    fn test_validate_device_index_valid() {
        let devices = vec![
            AudioDevice {
                index: 0,
                name: "Device A".to_string(),
                is_default: true,
                sample_rate: 48000,
                channels: 2,
            },
            AudioDevice {
                index: 1,
                name: "Device B".to_string(),
                is_default: false,
                sample_rate: 44100,
                channels: 2,
            },
        ];

        let result = validate_device_index(0, &devices, DeviceDirection::Input);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().name, "Device A");

        let result = validate_device_index(1, &devices, DeviceDirection::Input);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().name, "Device B");
    }

    #[test]
    fn test_validate_device_index_out_of_bounds() {
        let devices = vec![AudioDevice {
            index: 0,
            name: "Device A".to_string(),
            is_default: true,
            sample_rate: 48000,
            channels: 2,
        }];

        let result = validate_device_index(5, &devices, DeviceDirection::Input);
        assert!(result.is_err());

        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Invalid input device index 5"));
        assert!(err_msg.contains("Available 1 devices"));
    }

    #[test]
    fn test_validate_device_index_empty_list() {
        let devices: Vec<AudioDevice> = vec![];

        let result = validate_device_index(0, &devices, DeviceDirection::Output);
        assert!(result.is_err());

        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Invalid output device index 0"));
        assert!(err_msg.contains("Available 0 devices"));
    }

    #[test]
    fn test_validate_device_index_output_direction_in_error() {
        let devices: Vec<AudioDevice> = vec![];

        let result = validate_device_index(0, &devices, DeviceDirection::Output);
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("output"));
    }
}
