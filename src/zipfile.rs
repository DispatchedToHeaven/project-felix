//! Zip files, small and plain: reading entries stored or deflated (what any zip tool makes),
//! writing entries stored. No mod API types here; the inflate and CRC functions come in.

pub struct Entry {
    pub name: String,
    pub data: Vec<u8>,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Every file in the zip `bytes` (folders left out), at most `max` bytes each once unpacked.
pub fn read(bytes: &[u8], inflate: &dyn Fn(&[u8]) -> Result<Vec<u8>, String>, max: usize) -> Result<Vec<Entry>, String> {
    // The end-of-central-directory record, searched from the end (a comment may follow it).
    let end = (0..bytes.len().saturating_sub(21)).rev().find(|&i| u32_at(bytes, i) == Some(0x0605_4b50)).ok_or("not a zip file")?;
    let count = u16_at(bytes, end + 10).ok_or("cut short")? as usize;
    let mut at = u32_at(bytes, end + 16).ok_or("cut short")? as usize;
    let mut out = Vec::new();
    for _ in 0..count {
        if u32_at(bytes, at) != Some(0x0201_4b50) {
            return Err("broken central directory".into());
        }
        let method = u16_at(bytes, at + 10).ok_or("cut short")?;
        let packed = u32_at(bytes, at + 20).ok_or("cut short")? as usize;
        let size = u32_at(bytes, at + 24).ok_or("cut short")? as usize;
        let name_len = u16_at(bytes, at + 28).ok_or("cut short")? as usize;
        let extra = u16_at(bytes, at + 30).ok_or("cut short")? as usize;
        let comment = u16_at(bytes, at + 32).ok_or("cut short")? as usize;
        let local = u32_at(bytes, at + 42).ok_or("cut short")? as usize;
        let name = String::from_utf8_lossy(bytes.get(at + 46..at + 46 + name_len).ok_or("cut short")?).replace('\\', "/");
        at += 46 + name_len + extra + comment;
        if name.ends_with('/') {
            continue;
        }
        if size > max {
            return Err(format!("{name} is too big"));
        }
        if u32_at(bytes, local) != Some(0x0403_4b50) {
            return Err(format!("{name}: broken local header"));
        }
        let start = local + 30 + u16_at(bytes, local + 26).ok_or("cut short")? as usize + u16_at(bytes, local + 28).ok_or("cut short")? as usize;
        let raw = bytes.get(start..start + packed).ok_or_else(|| format!("{name}: cut short"))?;
        let data = match method {
            0 => raw.to_vec(),
            8 => inflate(raw)?,
            m => return Err(format!("{name}: packed in a way this can't read ({m})")),
        };
        if data.len() != size {
            return Err(format!("{name}: wrong size"));
        }
        out.push(Entry { name, data });
    }
    Ok(out)
}

/// A zip of `entries`, stored (not packed: they are small, and PNGs are packed already).
/// Midnight, 1 January 1980 (DOS time and date): the earliest a zip can say.
const DATE: [u8; 4] = [0, 0, 0x21, 0];

pub fn write(entries: &[Entry], crc32: &dyn Fn(&[u8]) -> u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for e in entries {
        let offset = out.len() as u32;
        let crc = crc32(&e.data);
        let name = e.name.as_bytes();
        let size = e.data.len() as u32;
        // Local header: version 2.0, no flags, stored, 1 January 1980, CRC, sizes, name.
        out.extend(0x0403_4b50u32.to_le_bytes());
        out.extend(20u16.to_le_bytes());
        out.extend([0u8; 4]);
        out.extend(DATE);
        out.extend(crc.to_le_bytes());
        out.extend(size.to_le_bytes());
        out.extend(size.to_le_bytes());
        out.extend((name.len() as u16).to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out.extend(name);
        out.extend(&e.data);
        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend(20u16.to_le_bytes());
        central.extend(20u16.to_le_bytes());
        central.extend([0u8; 4]);
        central.extend(DATE);
        central.extend(crc.to_le_bytes());
        central.extend(size.to_le_bytes());
        central.extend(size.to_le_bytes());
        central.extend((name.len() as u16).to_le_bytes());
        central.extend([0u8; 12]);
        central.extend(offset.to_le_bytes());
        central.extend(name);
    }
    let dir_at = out.len() as u32;
    let dir_len = central.len() as u32;
    out.extend(central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0u8; 4]);
    out.extend((entries.len() as u16).to_le_bytes());
    out.extend((entries.len() as u16).to_le_bytes());
    out.extend(dir_len.to_le_bytes());
    out.extend(dir_at.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crc(d: &[u8]) -> u32 {
        // Plain CRC-32, so the tests need no crate.
        let mut c = 0xFFFF_FFFFu32;
        for &b in d {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }

    #[test]
    fn round_trip_and_folders() {
        let z = write(&[Entry { name: "npc.json".into(), data: b"{\"id\":\"bob\"}".to_vec() }, Entry { name: "skin.png".into(), data: vec![1, 2, 3] }], &crc);
        let back = read(&z, &|_| Err("no deflate here".into()), 1 << 20).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].name, "npc.json");
        assert_eq!(back[1].data, vec![1, 2, 3]);
        assert_eq!(crc(b"123456789"), 0xCBF4_3926);
        assert!(read(b"not a zip at all, really not", &|_| Ok(vec![]), 10).is_err());
        assert!(read(&z, &|_| Ok(vec![]), 2).is_err()); // over the size cap
    }
}
