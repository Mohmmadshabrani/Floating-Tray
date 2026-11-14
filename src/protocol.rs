//! Wire format used by the vendored YASB Explorer hook.

const HEADER: usize = 28;
const MAX_ICON_SIDE: usize = 256;
pub const MAX_PACKET: usize = HEADER + 4096 + MAX_ICON_SIDE * MAX_ICON_SIDE * 4;
pub const ADD: u32 = 0;
pub const MODIFY: u32 = 1;
pub const DELETE: u32 = 2;
pub const SET_VERSION: u32 = 4;
pub const MESSAGE: u32 = 1;
pub const IMAGE: u32 = 2;
pub const TIP: u32 = 4;
pub const STATE: u32 = 8;
pub const GUID: u32 = 32;

pub struct IconImage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

pub struct Change {
    pub operation: u32,
    pub window: u32,
    pub id: u32,
    pub flags: u32,
    pub callback: u32,
    pub tooltip: String,
    pub hidden: bool,
    pub version: Option<u32>,
    pub guid: Option<[u8; 16]>,
    pub image: Option<IconImage>,
}

pub enum Packet {
    Ready,
    Change(Change),
}

pub fn parse(bytes: &[u8]) -> Option<Packet> {
    match word(bytes, 0)? {
        1 if bytes
            .get(4..)?
            .starts_with(b"[DLL] Successfully subclassed Shell_TrayWnd") =>
        {
            Some(Packet::Ready)
        }
        2 => parse_change(bytes).map(Packet::Change),
        _ => None,
    }
}

fn parse_change(bytes: &[u8]) -> Option<Change> {
    if bytes.len() > MAX_PACKET || bytes.get(4..12)? != 1u64.to_le_bytes() {
        return None;
    }
    let payload_len = word(bytes, 12)? as usize;
    let (width, height, image_len) = (
        word(bytes, 16)? as usize,
        word(bytes, 20)? as usize,
        word(bytes, 24)? as usize,
    );
    let payload_end = HEADER.checked_add(payload_len)?;
    let payload = bytes.get(HEADER..payload_end)?;
    if payload.len() < 8 {
        return None;
    }
    let operation = word(payload, 4)?;
    if !matches!(operation, ADD | MODIFY | DELETE | SET_VERSION) {
        return None;
    }
    let structure = payload.get(8..)?;
    let structure = structure.get(..structure.len().min(word(structure, 0)? as usize))?;
    let flags = word(structure, 12)?;
    let (window, id, callback) = (
        word(structure, 4)?,
        word(structure, 8)?,
        word(structure, 16)?,
    );
    let version = (operation == SET_VERSION)
        .then(|| word(structure, 800))
        .flatten();
    let guid = if flags & GUID != 0 {
        structure
            .get(936..952)
            .and_then(|v| <[u8; 16]>::try_from(v).ok())
            .filter(|value| value.iter().any(|&byte| byte != 0))
    } else {
        None
    };
    let tooltip = if flags & TIP != 0 {
        let end = if structure.len() <= 152 { 152 } else { 280 }.min(structure.len());
        let units = structure
            .get(24..end)?
            .chunks_exact(2)
            .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
            .take_while(|&unit| unit != 0);
        String::from_utf16_lossy(&units.collect::<Vec<_>>())
    } else {
        String::new()
    };
    let image = if flags & IMAGE != 0 && image_len != 0 {
        if width == 0
            || height == 0
            || width > MAX_ICON_SIDE
            || height > MAX_ICON_SIDE
            || image_len != width * height * 4
            || bytes.len() != payload_end.checked_add(image_len)?
        {
            return None;
        }
        Some(resize_icon(bytes.get(payload_end..)?, width, height))
    } else {
        None
    };
    let hidden = word(structure, 280).unwrap_or_default() & 1 != 0;
    let flags = if operation == MODIFY && word(structure, 284).unwrap_or_default() & 1 == 0 {
        flags & !STATE
    } else {
        flags
    };
    Some(Change {
        operation,
        window,
        id,
        callback,
        flags,
        tooltip,
        hidden,
        version,
        guid,
        image,
    })
}

fn resize_icon(rgba: &[u8], width: usize, height: usize) -> IconImage {
    const MAX_STORED_SIDE: usize = 64;
    let longest = width.max(height);
    if longest <= MAX_STORED_SIDE {
        return IconImage {
            width,
            height,
            rgba: rgba.to_vec(),
        };
    }
    let out_width = (width * MAX_STORED_SIDE / longest).max(1);
    let out_height = (height * MAX_STORED_SIDE / longest).max(1);
    let mut out = Vec::with_capacity(4 * out_width * out_height);
    for y in 0..out_height {
        for x in 0..out_width {
            let offset = 4 * (y * height / out_height * width + x * width / out_width);
            out.extend_from_slice(&rgba[offset..offset + 4]);
        }
    }
    IconImage {
        width: out_width,
        height: out_height,
        rgba: out,
    }
}

fn word(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(packet: &mut [u8], offset: usize, value: u32) {
        packet[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn sample(operation: u32, flags: u32, pixels: &[u8]) -> Vec<u8> {
        let mut packet = vec![0; HEADER + 8 + 956];
        put(&mut packet, 0, 2);
        put(&mut packet, 4, 1);
        put(&mut packet, 12, 964);
        if !pixels.is_empty() {
            put(&mut packet, 16, 1);
            put(&mut packet, 20, 1);
            put(&mut packet, 24, pixels.len() as u32);
        }
        put(&mut packet, HEADER, 0x3475_3432);
        put(&mut packet, HEADER + 4, operation);
        let start = HEADER + 8;
        put(&mut packet, start, 956);
        put(&mut packet, start + 4, 17);
        put(&mut packet, start + 8, 4);
        put(&mut packet, start + 12, flags);
        put(&mut packet, start + 16, 0x8011);
        packet[start + 24..start + 26].copy_from_slice(&(b'S' as u16).to_le_bytes());
        put(&mut packet, start + 800, 4);
        packet[start + 936] = 7;
        packet.extend_from_slice(pixels);
        packet
    }

    #[test]
    fn decodes_live_icon_and_version_packets() {
        let data = sample(MODIFY, MESSAGE | IMAGE | TIP | GUID, &[255, 0, 0, 255]);
        let Some(Packet::Change(change)) = parse(&data) else {
            panic!("missing icon")
        };
        assert_eq!((change.window, change.id, change.callback), (17, 4, 0x8011));
        assert_eq!(change.tooltip, "S");
        assert_eq!(change.guid.unwrap()[0], 7);
        assert_eq!(change.image.unwrap().rgba, [255, 0, 0, 255]);
        let Some(Packet::Change(change)) = parse(&sample(SET_VERSION, 0, &[])) else {
            panic!("missing version")
        };
        assert_eq!(change.version, Some(4));
    }

    #[test]
    fn rejects_truncated_image_and_unrelated_copydata() {
        let mut bytes = sample(ADD, IMAGE, &[5, 6, 7, 8]);
        bytes.pop();
        assert!(parse(&bytes).is_none());
        put(&mut bytes, 4, 2);
        assert!(parse(&bytes).is_none());
    }
}
