use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, SampleFormat};

use crate::config::DeviceNames;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Role {
    Mic,
    Cable,
    Monitor,
}

pub const ROLES: [Role; 3] = [Role::Mic, Role::Cable, Role::Monitor];

#[derive(Clone, PartialEq, Debug, Default)]
pub enum DeviceState {
    #[default]
    NotSelected,
    Open,
    Missing,
    Failed(String),
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct DeviceStatus {
    pub mic: DeviceState,
    pub cable: DeviceState,
    pub monitor: DeviceState,
}

impl DeviceStatus {
    pub fn get(&self, role: Role) -> &DeviceState {
        match role {
            Role::Mic => &self.mic,
            Role::Cable => &self.cable,
            Role::Monitor => &self.monitor,
        }
    }

    pub fn set(&mut self, role: Role, state: DeviceState) {
        match role {
            Role::Mic => self.mic = state,
            Role::Cable => self.cable = state,
            Role::Monitor => self.monitor = state,
        }
    }
}

pub fn selected(names: &DeviceNames, role: Role) -> Option<&str> {
    match role {
        Role::Mic => names.mic.as_deref(),
        Role::Cable => names.cable.as_deref(),
        Role::Monitor => names.monitor.as_deref(),
    }
}

pub fn list_inputs() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|it| it.map(|d| d.to_string()).collect())
        .unwrap_or_default()
}

pub fn list_outputs() -> Vec<String> {
    cpal::default_host()
        .output_devices()
        .map(|it| it.map(|d| d.to_string()).collect())
        .unwrap_or_default()
}

/// The VB-Cable playback device ("CABLE Input …"), if installed.
pub fn find_cable(outputs: &[String]) -> Option<String> {
    outputs
        .iter()
        .find(|n| n.to_lowercase().contains("cable input"))
        .cloned()
}

/// Decide which roles need (re)opening now, and the status of the rest.
/// A remembered device that isn't present is `Missing` (not cleared), so it comes back when plugged in.
pub fn plan_reconnect(
    selected_names: &DeviceNames,
    inputs: &[String],
    outputs: &[String],
    status: &DeviceStatus,
    lost: &[Role],
) -> (Vec<Role>, DeviceStatus) {
    let mut to_open = Vec::new();
    let mut next = status.clone();
    for role in ROLES {
        let Some(name) = selected(selected_names, role) else {
            next.set(role, DeviceState::NotSelected);
            continue;
        };
        let list = if role == Role::Mic { inputs } else { outputs };
        let present = list.iter().any(|n| n == name);
        let was_lost = lost.contains(&role);
        match (present, status.get(role)) {
            (false, _) => next.set(role, DeviceState::Missing),
            (true, DeviceState::Open) if !was_lost => {}
            (true, _) => to_open.push(role),
        }
    }
    (to_open, next)
}

fn find_device(name: &str, input: bool) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    let devices = if input {
        host.input_devices()
    } else {
        host.output_devices()
    };
    devices
        .map_err(|e| e.to_string())?
        .find(|d| d.to_string() == name)
        .ok_or_else(|| "Not found".to_string())
}

/// Stream errors that mean the device is gone or the stream is dead; xruns are survivable.
fn error_handler(lost: Arc<AtomicBool>) -> impl FnMut(cpal::Error) + Send + 'static {
    move |e| {
        if !matches!(e.kind(), ErrorKind::Xrun | ErrorKind::RealtimeDenied) {
            lost.store(true, Ordering::Relaxed);
        }
    }
}

/// Starting capacity for i16 conversion buffers; grows (rarely) if a callback is larger.
const SCRATCH_SAMPLES: usize = 16_384;

/// Open a capture stream on the named input device. `on_samples(interleaved, channels)`.
pub fn open_input(
    name: &str,
    mut on_samples: impl FnMut(&[f32], usize) + Send + 'static,
    lost: Arc<AtomicBool>,
) -> Result<(cpal::Stream, u32), String> {
    let device = find_device(name, true)?;
    let supported = device.default_input_config().map_err(|e| e.to_string())?;
    let config = supported.config();
    let channels = config.channels as usize;
    let rate = config.sample_rate;
    let stream = match supported.sample_format() {
        SampleFormat::F32 => device.build_input_stream::<f32, _, _>(
            config,
            move |data, _| on_samples(data, channels),
            error_handler(lost),
            None,
        ),
        SampleFormat::I16 => {
            let mut scratch = Vec::with_capacity(SCRATCH_SAMPLES);
            device.build_input_stream::<i16, _, _>(
                config,
                move |data, _| {
                    scratch.clear();
                    scratch.extend(data.iter().map(|&s| s as f32 / 32_768.0));
                    on_samples(&scratch, channels)
                },
                error_handler(lost),
                None,
            )
        }
        other => return Err(format!("Unsupported sample format {other}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, rate))
}

/// Open a playback stream on the named output device. `fill(interleaved, channels)`.
pub fn open_output(
    name: &str,
    mut fill: impl FnMut(&mut [f32], usize) + Send + 'static,
    lost: Arc<AtomicBool>,
) -> Result<(cpal::Stream, u32), String> {
    let device = find_device(name, false)?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let config = supported.config();
    let channels = config.channels as usize;
    let rate = config.sample_rate;
    let stream = match supported.sample_format() {
        SampleFormat::F32 => device.build_output_stream::<f32, _, _>(
            config,
            move |data, _| fill(data, channels),
            error_handler(lost),
            None,
        ),
        SampleFormat::I16 => {
            let mut scratch = vec![0.0f32; SCRATCH_SAMPLES];
            device.build_output_stream::<i16, _, _>(
                config,
                move |data, _| {
                    scratch.resize(data.len(), 0.0);
                    fill(&mut scratch, channels);
                    for (d, s) in data.iter_mut().zip(&scratch) {
                        *d = (s.clamp(-1.0, 1.0) * 32_767.0) as i16;
                    }
                },
                error_handler(lost),
                None,
            )
        }
        other => return Err(format!("Unsupported sample format {other}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(mic: Option<&str>, cable: Option<&str>, monitor: Option<&str>) -> DeviceNames {
        DeviceNames {
            mic: mic.map(Into::into),
            cable: cable.map(Into::into),
            monitor: monitor.map(Into::into),
        }
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn find_cable_matches_vb_name() {
        let outs = s(&["Speakers (Realtek)", "CABLE Input (VB-Audio Virtual Cable)"]);
        assert_eq!(
            find_cable(&outs).as_deref(),
            Some("CABLE Input (VB-Audio Virtual Cable)")
        );
        assert_eq!(find_cable(&s(&["Speakers"])), None);
        assert_eq!(
            find_cable(&s(&["cable input (vb-audio)"])).as_deref(),
            Some("cable input (vb-audio)")
        );
    }

    #[test]
    fn missing_at_startup() {
        let (open, st) = plan_reconnect(
            &names(Some("USB Mic"), None, None),
            &s(&["Realtek Mic"]),
            &[],
            &DeviceStatus::default(),
            &[],
        );
        assert_eq!(st.mic, DeviceState::Missing);
        assert!(!open.contains(&Role::Mic));
    }

    #[test]
    fn present_at_startup_opens() {
        let (open, _) = plan_reconnect(
            &names(Some("USB Mic"), Some("CABLE Input"), Some("Headphones")),
            &s(&["USB Mic"]),
            &s(&["CABLE Input", "Headphones"]),
            &DeviceStatus::default(),
            &[],
        );
        assert_eq!(open, vec![Role::Mic, Role::Cable, Role::Monitor]);
    }

    #[test]
    fn reappears_reopens() {
        let status = DeviceStatus {
            mic: DeviceState::Missing,
            ..Default::default()
        };
        let (open, _) = plan_reconnect(
            &names(Some("USB Mic"), None, None),
            &s(&["USB Mic"]),
            &[],
            &status,
            &[],
        );
        assert_eq!(open, vec![Role::Mic]);
    }

    #[test]
    fn failed_is_retried() {
        let status = DeviceStatus {
            cable: DeviceState::Failed("busy".into()),
            ..Default::default()
        };
        let (open, _) = plan_reconnect(
            &names(None, Some("CABLE Input"), None),
            &[],
            &s(&["CABLE Input"]),
            &status,
            &[],
        );
        assert_eq!(open, vec![Role::Cable]);
    }

    #[test]
    fn lost_flag_reopens_or_marks_missing() {
        let status = DeviceStatus {
            cable: DeviceState::Open,
            ..Default::default()
        };
        let sel = names(None, Some("CABLE Input"), None);
        let (open, _) = plan_reconnect(&sel, &[], &s(&["CABLE Input"]), &status, &[Role::Cable]);
        assert_eq!(open, vec![Role::Cable]);

        let (open, st) = plan_reconnect(&sel, &[], &s(&["Speakers"]), &status, &[Role::Cable]);
        assert!(open.is_empty());
        assert_eq!(st.cable, DeviceState::Missing);
    }

    #[test]
    fn not_selected_stays_quiet() {
        let status = DeviceStatus {
            monitor: DeviceState::Open,
            ..Default::default()
        };
        let (open, st) = plan_reconnect(
            &names(None, None, None),
            &[],
            &s(&["Headphones"]),
            &status,
            &[],
        );
        assert!(open.is_empty());
        assert_eq!(st.monitor, DeviceState::NotSelected);
    }

    #[test]
    fn open_and_healthy_untouched() {
        let status = DeviceStatus {
            mic: DeviceState::Open,
            ..Default::default()
        };
        let (open, st) = plan_reconnect(
            &names(Some("USB Mic"), None, None),
            &s(&["USB Mic"]),
            &[],
            &status,
            &[],
        );
        assert!(open.is_empty());
        assert_eq!(st.mic, DeviceState::Open);
    }

    #[test]
    #[ignore = "prints this machine's devices"]
    fn lists_real_devices() {
        println!("inputs: {:#?}", list_inputs());
        println!("outputs: {:#?}", list_outputs());
    }
}
