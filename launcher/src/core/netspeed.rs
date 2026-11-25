pub fn octets() -> Option<(u64, u64)> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{GetBestInterface, GetIfEntry2, MIB_IF_ROW2};
    let mut idx = 0u32;
    if unsafe { GetBestInterface(u32::from_ne_bytes([8, 8, 8, 8]), &mut idx) } != 0 {
        return None;
    }
    let mut row: MIB_IF_ROW2 = unsafe { std::mem::zeroed() };
    row.InterfaceIndex = idx;
    if unsafe { GetIfEntry2(&mut row) } != 0 {
        return None;
    }
    Some((row.InOctets, row.OutOctets))
}

pub fn mbit(bytes_per_sec: f64) -> String {
    let m = bytes_per_sec * 8.0 / 1_000_000.0;
    if m >= 10.0 {
        format!("{m:.0} Мбит/с")
    } else if m >= 0.1 {
        format!("{m:.1} Мбит/с").replace('.', ",")
    } else {
        format!("{:.0} Кбит/с", bytes_per_sec * 8.0 / 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speeds_read_simply() {
        assert_eq!(mbit(2_500_000.0), "20 Мбит/с");
        assert_eq!(mbit(150_000.0), "1,2 Мбит/с");
        assert_eq!(mbit(5_000.0), "40 Кбит/с");
    }

    #[test]
    #[ignore]
    fn live_octets() {
        println!("{:?}", octets());
    }
}
