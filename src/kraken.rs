//! NZXT Kraken control.
//!
//! Two interfaces are involved. Commands, status and fan/pump duty go over the
//! HID interface as 64-byte reports, where the leading command byte doubles as
//! the HID report id. LCD images go over a separate WinUSB bulk endpoint, in
//! chunks, after reserving space in one of the device's 16 image "buckets".
//!
//! The wire protocol follows liquidctl's `kraken3` driver.

use std::time::Duration;

const REPORT_LEN: usize = 64;
const BULK_ENDPOINT: u8 = 0x02;
const BULK_INTERFACE: u8 = 0;
/// Total image memory on the device, in 1 KB pages.
const LCD_TOTAL_MEMORY: u32 = 24320;
/// Duty curves are transmitted as one byte per degree across this range.
const CURVE_MIN_TEMP: u8 = 20;
const CURVE_CRITICAL_TEMP: u8 = 59;

/// A supported cooler and the few numbers that differ between models.
pub struct Model {
    pub vid: u16,
    pub pid: u16,
    pub name: &'static str,
    pub resolution: (u32, u32),
    pub bulk_chunk: usize,
    /// Channel id, minimum duty.
    pub pump: ([u8; 3], u8),
    pub fan: ([u8; 3], u8),
    /// The 2023+ coolers moved to a different image upload path on firmware 2.
    pub modern: bool,
}

pub const MODELS: &[Model] = &[
    Model {
        vid: 0x1E71,
        pid: 0x3008,
        name: "NZXT Kraken Z (Z53, Z63 or Z73)",
        resolution: (320, 320),
        bulk_chunk: 512,
        pump: ([0x1, 0x0, 0x0], 20),
        fan: ([0x2, 0x0, 0x0], 0),
        modern: false,
    },
    Model {
        vid: 0x1E71,
        pid: 0x300C,
        name: "NZXT Kraken 2023 Elite",
        resolution: (640, 640),
        bulk_chunk: 2 * 1024 * 1024,
        pump: ([0x1, 0x1, 0x0], 20),
        fan: ([0x2, 0x1, 0x1], 0),
        modern: true,
    },
    Model {
        vid: 0x1E71,
        pid: 0x300E,
        name: "NZXT Kraken 2023",
        resolution: (240, 240),
        bulk_chunk: 2 * 1024 * 1024,
        pump: ([0x1, 0x1, 0x0], 20),
        fan: ([0x2, 0x1, 0x1], 0),
        modern: true,
    },
    Model {
        vid: 0x1E71,
        pid: 0x3012,
        name: "NZXT Kraken 2024 Elite RGB",
        resolution: (640, 640),
        bulk_chunk: 2 * 1024 * 1024,
        pump: ([0x1, 0x1, 0x0], 20),
        fan: ([0x2, 0x1, 0x1], 0),
        modern: true,
    },
    Model {
        vid: 0x1E71,
        pid: 0x3014,
        name: "NZXT Kraken 2024 Plus",
        resolution: (240, 240),
        bulk_chunk: 2 * 1024 * 1024,
        pump: ([0x1, 0x1, 0x0], 20),
        fan: ([0x2, 0x1, 0x1], 0),
        modern: true,
    },
];

pub struct Kraken {
    hid: hidapi::HidDevice,
    /// Only opened when an image is going to be sent.
    bulk: Option<rusb::DeviceHandle<rusb::GlobalContext>>,
    pub model: &'static Model,
    pub serial: String,
    /// Reused pixel staging buffer, so a frame costs no allocation.
    scratch: Vec<u8>,
}

pub struct Status {
    pub liquid_temp: f32,
    pub pump_rpm: u16,
    pub pump_duty: u8,
    pub fan_rpm: u16,
    pub fan_duty: u8,
}

impl Kraken {
    /// Find and open the configured cooler, or the first supported one.
    pub fn open(cfg: &crate::config::Device) -> Result<Self, String> {
        let api = hidapi::HidApi::new().map_err(|e| format!("HID init failed: {e}"))?;

        let mut chosen: Option<(&'static Model, String)> = None;
        for info in api.device_list() {
            let Some(model) = MODELS
                .iter()
                .find(|m| m.vid == info.vendor_id() && m.pid == info.product_id())
            else {
                continue;
            };
            if cfg.vendor_id != 0 && cfg.vendor_id != model.vid {
                continue;
            }
            if cfg.product_id != 0 && cfg.product_id != model.pid {
                continue;
            }
            let serial = info.serial_number().unwrap_or("").to_string();
            if !cfg.serial.is_empty() && !serial.eq_ignore_ascii_case(&cfg.serial) {
                continue;
            }
            // The Kraken exposes several HID collections; the vendor-defined
            // one carries the control protocol. Prefer usage page 0xFF00 and
            // fall back to whatever matched.
            let vendor_defined = info.usage_page() >= 0xFF00;
            if chosen.is_none() || vendor_defined {
                chosen = Some((model, serial));
                if vendor_defined {
                    break;
                }
            }
        }

        let (model, serial) = chosen.ok_or_else(|| {
            "no supported NZXT cooler found (close NZXT CAM and check the USB header)".to_string()
        })?;

        let hid = api
            .open_serial(model.vid, model.pid, &serial)
            .or_else(|_| api.open(model.vid, model.pid))
            .map_err(|e| format!("could not open {} over HID: {e}", model.name))?;

        Ok(Kraken { hid, bulk: None, model, serial, scratch: Vec::new() })
    }

    // --- HID plumbing ------------------------------------------------------

    /// Send a 64-byte report, zero-padded. `data[0]` is the command byte.
    fn write(&self, data: &[u8]) -> Result<(), String> {
        let mut buf = [0u8; REPORT_LEN];
        let n = data.len().min(REPORT_LEN);
        buf[..n].copy_from_slice(&data[..n]);
        self.hid.write(&buf).map_err(|e| format!("HID write failed: {e}"))?;
        Ok(())
    }

    fn read(&self) -> Result<[u8; REPORT_LEN], String> {
        let mut buf = [0u8; REPORT_LEN];
        let n = self
            .hid
            .read_timeout(&mut buf, 1000)
            .map_err(|e| format!("HID read failed: {e}"))?;
        if n == 0 {
            return Err("timed out waiting for the cooler to reply".into());
        }
        Ok(buf)
    }

    /// Write, then read until a report with the expected two-byte prefix
    /// arrives. The device interleaves unsolicited status reports, so replies
    /// have to be filtered rather than simply read once.
    fn write_then_read(&self, data: &[u8], prefix: [u8; 2]) -> Result<[u8; REPORT_LEN], String> {
        self.write(data)?;
        for _ in 0..12 {
            let msg = self.read()?;
            if msg[0] == prefix[0] && msg[1] == prefix[1] {
                return Ok(msg);
            }
        }
        Err(format!("no {:02X?} reply from the cooler", prefix))
    }

    // --- Status and speed --------------------------------------------------

    pub fn status(&self) -> Result<Status, String> {
        let msg = self.write_then_read(&[0x74, 0x01], [0x75, 0x01])?;
        Ok(Status {
            liquid_temp: msg[15] as f32 + msg[16] as f32 / 10.0,
            pump_rpm: u16::from_le_bytes([msg[17], msg[18]]),
            pump_duty: msg[19],
            fan_rpm: u16::from_le_bytes([msg[23], msg[24]]),
            fan_duty: msg[25],
        })
    }

    /// Upload a duty curve for one channel. The firmware interpolates over
    /// liquid temperature between 20 and 59 C, one byte per degree.
    pub fn set_curve(&self, channel: Channel, points: &[[u8; 2]]) -> Result<(), String> {
        let (cid, dmin) = match channel {
            Channel::Pump => self.model.pump,
            Channel::Fan => self.model.fan,
        };
        let curve = build_curve(points, dmin, 100);
        let mut msg = Vec::with_capacity(4 + curve.len());
        msg.push(0x72);
        msg.extend_from_slice(&cid);
        msg.extend_from_slice(&curve);
        self.write(&msg)
    }

    // --- LCD ---------------------------------------------------------------

    pub fn set_brightness(&self, percent: u8) -> Result<(), String> {
        let (_, orientation) = self.lcd_info()?;
        self.write(&[0x30, 0x02, 0x01, percent.min(100), 0x0, 0x0, 0x1, orientation])
    }

    /// Returns (brightness, orientation index).
    fn lcd_info(&self) -> Result<(u8, u8), String> {
        let msg = self.write_then_read(&[0x30, 0x01], [0x31, 0x01])?;
        Ok((msg[0x18], msg[0x1A]))
    }

    /// Force the panel to orientation 0. Frames are rotated while rendering,
    /// so letting the device rotate too would double the effect.
    pub fn reset_orientation(&self) -> Result<(), String> {
        let (brightness, _) = self.lcd_info()?;
        self.write(&[0x30, 0x02, 0x01, brightness, 0x0, 0x0, 0x1, 0x0])
    }

    fn bulk(&mut self) -> Result<&rusb::DeviceHandle<rusb::GlobalContext>, String> {
        if self.bulk.is_none() {
            let (vid, pid) = (self.model.vid, self.model.pid);
            let devices = rusb::devices().map_err(|e| format!("USB enumeration failed: {e}"))?;
            let mut handle = None;
            for device in devices.iter() {
                let Ok(desc) = device.device_descriptor() else { continue };
                if desc.vendor_id() != vid || desc.product_id() != pid {
                    continue;
                }
                match device.open() {
                    Ok(h) => {
                        // Prefer the unit whose serial matches the HID handle,
                        // for setups with more than one cooler.
                        let matches = h
                            .read_serial_number_string_ascii(&desc)
                            .map(|s| s.eq_ignore_ascii_case(&self.serial))
                            .unwrap_or(false);
                        if matches || handle.is_none() {
                            handle = Some(h);
                        }
                        if matches {
                            break;
                        }
                    }
                    Err(_) => continue,
                }
            }
            let handle = handle.ok_or_else(|| {
                "could not open the cooler's bulk interface; it needs the WinUSB driver \
                 (NZXT CAM or Zadig binds it)"
                    .to_string()
            })?;
            handle
                .claim_interface(BULK_INTERFACE)
                .map_err(|e| format!("could not claim the bulk interface: {e}"))?;
            self.bulk = Some(handle);
        }
        Ok(self.bulk.as_ref().unwrap())
    }

    fn bulk_write(&mut self, data: &[u8]) -> Result<(), String> {
        let handle = self.bulk()?;
        handle
            .write_bulk(BULK_ENDPOINT, data, Duration::from_secs(5))
            .map_err(|e| format!("bulk write failed: {e}"))?;
        Ok(())
    }

    /// Send one still image. `rgba` is tightly packed RGBA at the panel's
    /// native resolution; the alpha byte is ignored by the device.
    pub fn set_image(&mut self, rgba: &[u8]) -> Result<(), String> {
        if self.model.modern {
            return Err(format!(
                "{} uses a different image protocol that CustomAIO does not implement yet",
                self.model.name
            ));
        }

        let (w, h) = self.model.resolution;
        let expected = (w * h * 4) as usize;
        if rgba.len() != expected {
            return Err(format!("expected {expected} bytes of pixel data, got {}", rgba.len()));
        }

        // The panel wants R, G, B, 0 per pixel. Staged in a buffer that is
        // allocated once and refilled in place on later frames.
        self.scratch.clear();
        self.scratch.extend_from_slice(rgba);
        for px in self.scratch.chunks_exact_mut(4) {
            px[3] = 0;
        }

        self.write_then_read(&[0x36, 0x03], [0x37, 0x03])?;

        let buckets = self.query_buckets()?;
        let free = buckets.iter().position(|b| !b[15..].iter().any(|&x| *&x != 0));
        let bucket = self.prepare_bucket(free.unwrap_or(0) as u8, free.is_none())?;

        let header: Vec<u8> = [
            0x12, 0xFA, 0x01, 0xE8, 0xAB, 0xCD, 0xEF, 0x98, 0x76, 0x54, 0x32, 0x10, 0x02, 0x0,
            0x0, 0x0,
        ]
        .iter()
        .copied()
        .chain((self.scratch.len() as u32).to_le_bytes())
        .collect();

        // Transfer size is counted in 1 KB pages.
        let pages = ((header.len() + self.scratch.len()) as u32).div_ceil(1024);
        let start = match bucket_memory_offset(&buckets, bucket as usize, pages) {
            Some(offset) => offset,
            None => {
                self.delete_all_buckets()?;
                0
            }
        };

        if !self.setup_bucket(bucket, bucket + 1, start, pages as u16)? {
            return Err("the cooler refused the image transfer setup".into());
        }

        self.write_then_read(&[0x36, 0x01, bucket], [0x37, 0x01])?;
        self.bulk_write(&header)?;
        // Take the buffer out so the bulk writes can borrow self mutably,
        // then put it back for the next frame.
        let staged = std::mem::take(&mut self.scratch);
        let chunk = self.model.bulk_chunk;
        let mut result = Ok(());
        for part in staged.chunks(chunk) {
            result = self.bulk_write(part);
            if result.is_err() {
                break;
            }
        }
        self.scratch = staged;
        result?;
        self.write(&[0x36, 0x02])?;

        if !self.switch_bucket(bucket, 0x4)? {
            return Err("the cooler refused to display the uploaded image".into());
        }
        Ok(())
    }

    fn query_buckets(&self) -> Result<Vec<[u8; REPORT_LEN]>, String> {
        let mut out = Vec::with_capacity(16);
        for i in 0..16u8 {
            out.push(self.write_then_read(&[0x30, 0x04, i], [0x31, 0x04])?);
        }
        Ok(out)
    }

    /// Free a bucket for reuse. A bucket that held data needs deleting twice,
    /// and a refusal means trying the next one.
    fn prepare_bucket(&self, index: u8, filled: bool) -> Result<u8, String> {
        let mut index = index;
        let mut filled = filled;
        loop {
            if index >= 16 {
                return Err("the cooler has no free image buckets".into());
            }
            if !self.delete_bucket(index)? {
                index += 1;
                filled = true;
                continue;
            }
            if filled {
                filled = false;
                continue;
            }
            return Ok(index);
        }
    }

    fn delete_bucket(&self, index: u8) -> Result<bool, String> {
        self.write(&[0x32, 0x02, index])?;
        for _ in 0..12 {
            let msg = self.read()?;
            if msg[0] == 0x33 && msg[1] == 0x02 {
                return Ok(msg[14] == 0x01);
            }
        }
        Ok(false)
    }

    fn delete_all_buckets(&self) -> Result<(), String> {
        self.switch_bucket(0, 0x2)?;
        for i in 0..16u8 {
            self.delete_bucket(i)?;
        }
        Ok(())
    }

    fn switch_bucket(&self, index: u8, mode: u8) -> Result<bool, String> {
        let msg = self.write_then_read(&[0x38, 0x01, mode, index], [0x39, 0x01])?;
        Ok(msg[14] == 0x01)
    }

    fn setup_bucket(&self, start: u8, end: u8, address: u16, pages: u16) -> Result<bool, String> {
        let a = address.to_le_bytes();
        let p = pages.to_le_bytes();
        let msg = self.write_then_read(
            &[0x32, 0x01, start, end, a[0], a[1], p[0], p[1], 0x01],
            [0x33, 0x01],
        )?;
        Ok(msg[14] == 0x01)
    }
}

#[derive(Clone, Copy)]
pub enum Channel {
    Pump,
    Fan,
}

/// Decide where in the device's image memory the next upload should land,
/// mirroring liquidctl: reuse the current bucket if the image still fits,
/// otherwise keep it if nothing overlaps, otherwise append after everything,
/// otherwise wrap to the start. `None` means the memory map has to be reset.
fn bucket_memory_offset(buckets: &[[u8; REPORT_LEN]], index: usize, pages: u32) -> Option<u16> {
    let read_u16 = |b: &[u8; REPORT_LEN], at: usize| u16::from_le_bytes([b[at], b[at + 1]]) as u32;

    let current = &buckets[index];
    let current_offset = read_u16(current, 17);
    let current_size = read_u16(current, 19);

    if pages <= current_size {
        return Some(current_offset as u16);
    }

    let mut min_occupied = current_offset;
    let mut max_occupied = 0u32;
    let mut overlaps = false;
    for (i, b) in buckets.iter().enumerate() {
        let start = read_u16(b, 17);
        let end = start + read_u16(b, 19);
        max_occupied = max_occupied.max(end);
        min_occupied = min_occupied.min(start);
        if (start > current_offset && start < current_offset + pages)
            || (start < current_offset && end > start)
            || (start == current_offset && i != index)
        {
            overlaps = true;
        }
    }

    if !overlaps {
        return Some(current_offset as u16);
    }
    if max_occupied + pages < LCD_TOTAL_MEMORY {
        return Some(max_occupied as u16);
    }
    if pages < min_occupied {
        return Some(0);
    }
    None
}

/// Expand `[temperature, duty]` points into one duty byte per degree from 20 to
/// 59 C, matching liquidctl's normalisation: sorted, monotonically increasing,
/// with a (59, 100) failsafe, then linearly interpolated.
fn build_curve(points: &[[u8; 2]], dmin: u8, dmax: u8) -> Vec<u8> {
    let mut profile: Vec<(i32, i32)> =
        points.iter().map(|p| (p[0] as i32, p[1] as i32)).collect();
    profile.push((CURVE_CRITICAL_TEMP as i32, 100));
    profile.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));

    let mut mono: Vec<(i32, i32)> = profile.first().copied().into_iter().collect();
    for &(x, mut y) in profile.iter().skip(1) {
        let (xb, yb) = *mono.last().unwrap();
        if x == xb {
            continue;
        }
        if y < yb {
            y = yb;
        }
        mono.push((x, y));
        if y == 100 {
            break;
        }
    }

    (CURVE_MIN_TEMP..=CURVE_CRITICAL_TEMP)
        .map(|t| {
            let duty = interpolate(&mono, t as i32);
            duty.clamp(dmin as i32, dmax as i32) as u8
        })
        .collect()
}

fn interpolate(profile: &[(i32, i32)], x: i32) -> i32 {
    let mut lower = profile[0];
    let mut upper = *profile.last().unwrap();
    for &step in profile {
        if step.0 <= x {
            lower = step;
        }
        if step.0 >= x {
            upper = step;
            break;
        }
    }
    if lower.0 == upper.0 {
        return lower.1;
    }
    // Round to nearest, as liquidctl does.
    let span = upper.0 - lower.0;
    let rise = upper.1 - lower.1;
    lower.1 + ((x - lower.0) * rise * 2 + span) / (span * 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolation_matches_liquidctl() {
        let p = vec![(20, 50), (50, 70), (60, 100)];
        assert_eq!(interpolate(&p, 33), 59);
        assert_eq!(interpolate(&[(20, 50), (50, 70)], 19), 50);
        assert_eq!(interpolate(&[(20, 50), (50, 70)], 51), 70);
        assert_eq!(interpolate(&[(20, 50)], 20), 50);
    }

    #[test]
    fn curve_covers_twenty_to_fiftynine() {
        let c = build_curve(&[[20, 30], [55, 100]], 0, 100);
        assert_eq!(c.len(), 40);
        assert_eq!(c[0], 30);
        assert_eq!(*c.last().unwrap(), 100);
    }

    #[test]
    fn curve_respects_channel_minimum() {
        // The pump refuses anything under 20%.
        let c = build_curve(&[[20, 0], [59, 0]], 20, 100);
        assert!(c.iter().all(|&d| d >= 20));
    }
}
